//! The Actions window (`dialog.actions`) through the control channel: an action run from the window gives the font
//! its command gives, the preview leaves the font as it was, a set built in the sidebar and saved runs with
//! `script.run` to the same font, and bad input changes nothing.

use std::sync::Arc;

use craft::host::Host;
use fontcraft_engine::Session;
use fontcraft_model::Font;
use serde_json::{Value, json};

use super::{GlyphScope, actions, matching, names_param};
use crate::app::{APP, FontCraftApp};
use crate::control;

fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
    control::handle(app, &json!({"id": 1, "method": method, "params": params}))
}

/// Runs `dialog.actions` with `params` and returns the window as `ui.inspect` reports it.
fn exec(app: &mut FontCraftApp, params: Value) -> Value {
    let r = call(app, "engine.execute", json!({"command": "dialog.actions", "params": params}));
    assert_eq!(r["ok"], true, "{params} {r}");
    call(app, "ui.inspect", json!({}))["result"]["actions"].clone()
}

fn refused(app: &mut FontCraftApp, params: Value) -> String {
    let r = call(app, "engine.execute", json!({"command": "dialog.actions", "params": params}));
    assert_eq!(r["ok"], false, "{params} {r}");
    r["error"].as_str().unwrap_or_default().to_string()
}

fn run(app: &mut FontCraftApp, command: &str, params: Value) {
    app.session.execute(command, &params).unwrap();
}

fn font(app: &FontCraftApp) -> Font {
    app.session.font().unwrap().clone()
}

fn masters(app: &FontCraftApp) -> Vec<String> {
    app.session.font().unwrap().masters.iter().map(|m| m.id.clone()).collect()
}

/// Two masters; A and B each hold two overlapping squares on both; A and B are selected in Font View.
fn app_with_outlines() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    run(&mut app, "font.master.add", json!({"name": "Bold"}));
    for master in masters(&app) {
        for glyph in ["A", "B"] {
            for at in [100, 200] {
                run(&mut app, "path.addRectangle", json!({"glyph": glyph, "layer": master, "x": at, "y": at, "width": 200, "height": 200}));
            }
        }
    }
    app.view.font_view.selected = vec!["A".into(), "B".into()];
    app
}

#[test]
fn dialog_actions_runs_three_actions_as_their_commands_do() {
    // (action, how its parameters are given, which form edits them)
    let cases = [
        ("filter.roundCorners", json!({"set": {"radius": 30}}), "dialog"),
        ("path.transformations", json!({"params": {"scale": {"x": 50}}}), "dialog"),
        ("path.removeOverlap", json!({"params": {"keepOpenPaths": false}}), "json"),
    ];
    for (command, given, form) in cases {
        let mut app = app_with_outlines();
        let before = font(&app);
        let r = exec(&mut app, json!({"command": command, "scope": "selected", "layers": "masters"}));
        assert_eq!(r["form"], form, "{command}");
        exec(&mut app, given);

        let r = exec(&mut app, json!({"preview": true}));
        assert_eq!(r["preview"]["changed"], true, "{command}: the preview shows a change {r}");
        assert_eq!(r["preview"]["glyph"], "A");
        assert_eq!(font(&app), before, "{command}: the preview leaves the font as it was");
        assert!(app.session.interaction().is_none());

        let params = r["params"].clone();
        let r = exec(&mut app, json!({"run": true}));
        assert_eq!((r["glyphs"].clone(), r["layerIds"].clone()), (json!(2), json!(masters(&app))), "{command}");
        let from_dialog = font(&app);
        assert_ne!(from_dialog, before, "{command} changed the font");

        // One undo step gives back the font; the command with the same parameters gives the window's font.
        run(&mut app, "edit.undo", json!({}));
        assert_eq!(font(&app), before, "{command}: one undo step");
        let mut direct = params.as_object().unwrap().clone();
        direct.insert("glyphs".into(), json!(["A", "B"]));
        direct.insert("layers".into(), json!(masters(&app)));
        run(&mut app, command, Value::Object(direct));
        assert_eq!(font(&app), from_dialog, "{command}: window equals command");
    }
}

#[test]
fn dialog_actions_saves_a_set_that_script_run_runs_to_the_same_font() {
    let dir = std::env::temp_dir().join(format!("fontcraft-ui-actions-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tidy.fcmacro").to_str().unwrap().to_string();
    let _ = std::fs::remove_file(&path);

    let mut app = app_with_outlines();
    let before = font(&app);
    exec(&mut app, json!({"command": "path.transformations", "params": {"scale": {"x": 50}}, "scope": "selected", "layers": "masters", "add": true}));
    exec(&mut app, json!({"command": "path.removeOverlap", "add": true}));
    exec(&mut app, json!({"command": "filter.roundCorners", "set": {"radius": 20}, "add": true}));
    let r = exec(&mut app, json!({"title": "Tidy"}));
    let steps = r["set"]["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0]["params"]["glyphs"], json!({"$arg": "glyphs"}));
    assert_eq!(steps[0]["params"]["layers"], json!({"$arg": "layers"}));
    assert_eq!(font(&app), before, "building a set changes nothing");

    exec(&mut app, json!({"savePath": path}));
    assert!(std::path::Path::new(&path).exists());
    let r = exec(&mut app, json!({"clear": true}));
    assert_eq!(r["set"]["steps"], json!([]));
    let r = exec(&mut app, json!({"openSet": path}));
    assert_eq!((r["set"]["title"].clone(), r["set"]["steps"].as_array().map(Vec::len)), (json!("Tidy"), Some(3)));

    exec(&mut app, json!({"runSet": true}));
    let from_set = font(&app);
    assert_ne!(from_set, before);
    run(&mut app, "edit.undo", json!({}));
    assert_eq!(font(&app), before, "the set is one undo step");

    let layers = masters(&app);
    run(&mut app, "script.run", json!({"macro": path, "glyphs": ["A", "B"], "layers": layers}));
    assert_eq!(font(&app), from_set, "the saved set runs with script.run to the same font");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dialog_actions_refuses_bad_input_and_changes_nothing() {
    let mut app = app_with_outlines();
    let before = font(&app);
    exec(&mut app, json!({"command": "path.removeOverlap", "scope": "selected"}));
    let state = app.view.actions.clone();

    assert!(refused(&mut app, json!({"command": "glyph.newGlyph"})).contains("not an action"));
    assert!(refused(&mut app, json!({"command": "nope.nothing"})).contains("unknown command"));
    assert!(refused(&mut app, json!({"params": {"radius": 3}})).contains("not a parameter"));
    assert!(refused(&mut app, json!({"set": {"glyphs": ["A"]}})).contains("comes from the scope"));
    assert!(refused(&mut app, json!({"params": [1]})).contains("JSON object"));
    assert!(refused(&mut app, json!({"scope": "everything"})).contains("scope"));
    assert!(refused(&mut app, json!({"layers": "some"})).contains("layers"));
    assert!(refused(&mut app, json!({"previewGlyph": "nope"})).contains("no glyph"));
    assert!(refused(&mut app, json!({"remove": 0})).contains("no step"));
    assert!(refused(&mut app, json!({"runSet": true})).contains("empty"));
    assert!(refused(&mut app, json!({"run": true, "runSet": true})).contains("at most one"));
    assert!(refused(&mut app, json!({"scope": "category", "category": "", "run": true})).contains("category"));
    assert!(refused(&mut app, json!({"openSet": "/no/such/file.fcmacro"})).contains("file.fcmacro"));
    assert!(refused(&mut app, json!({"openSet": "Nothing"})).contains("no macro named"));
    assert_eq!(app.view.actions, state, "a refused call leaves the window as it was");
    assert_eq!(font(&app), before);

    // The JSON editor keeps text as typed and reports why it is not parameters.
    let r = exec(&mut app, json!({"json": "{\"radius\": "}));
    assert!(r["error"].as_str().is_some_and(|e| e.contains("not JSON")), "{r}");

    // A form refuses a value out of its field's range, and `json`.
    exec(&mut app, json!({"command": "filter.roundCorners"}));
    assert!(refused(&mut app, json!({"set": {"radius": -5}})).contains("radius"));
    assert!(refused(&mut app, json!({"set": {"nope": 1}})).contains("no field"));
    assert!(refused(&mut app, json!({"json": "{}"})).contains("json"));
    assert_eq!(font(&app), before);
}

#[test]
fn dialog_actions_scopes_give_their_glyphs_and_layers() {
    let mut app = app_with_outlines();
    let r = exec(&mut app, json!({"command": "path.removeOverlap", "scope": "selected", "layers": "active"}));
    assert_eq!((r["glyphs"].clone(), r["layerIds"].clone()), (json!(2), json!([masters(&app)[0]])));
    let r = exec(&mut app, json!({"scope": "font"}));
    assert_eq!(r["glyphs"], json!(app.session.font().unwrap().glyphs.len()));
    let r = exec(&mut app, json!({"scope": "category", "category": "letter"}));
    let letters = r["glyphs"].as_u64().unwrap();
    assert!(letters >= 2 && letters < app.session.font().unwrap().glyphs.len() as u64, "{r}");
    app.view.glyph = Some("B".into());
    let r = exec(&mut app, json!({"scope": "glyph"}));
    assert_eq!(r["glyphs"], 1);

    // A command that takes one `layer` runs once per layer.
    let one_layer = actions()
        .iter()
        .map(|a| fontcraft_engine::find_command(a.id).unwrap())
        .find(|s| !names_param(s.params, "layers") && !names_param(s.params, "allMasters") && names_param(s.params, "layer"));
    if let Some(spec) = one_layer {
        let r = exec(&mut app, json!({"command": spec.id, "layers": "masters"}));
        assert_eq!(r["steps"], json!(masters(&app).len()), "{}", spec.id);
    }
}

#[test]
fn dialog_actions_lists_editing_commands_that_take_glyphs() {
    let list = actions();
    let ids: Vec<&str> = list.iter().map(|a| a.id).collect();
    for id in ["filter.roundCorners", "path.removeOverlap", "path.transformations"] {
        assert!(ids.contains(&id), "{id}");
    }
    for id in ["glyph.newGlyph", "script.run", "script.runActionSet", "edit.undo"] {
        assert!(!ids.contains(&id), "{id}");
    }
    assert!(matching(&list, "overlap").any(|a| a.id == "path.removeOverlap"));
    assert!(matching(&list, "zzzz-none").next().is_none());
    assert!(names_param("{glyphs: [names], layer?: id} → {glyphs}", "layer"));
    assert!(!names_param("{glyphs: [names], layers?: [ids]}", "layer"));
    assert!(!names_param("{x: 1} → {layer: id}", "layer"));
    assert_eq!(GlyphScope::parse("category"), Some(GlyphScope::Category));
}

/// The window opens on the dialog host and draws its preview filled and as outlines, the preview leaves no
/// interaction running, and closing the window drops the preview.
#[test]
fn dialog_actions_window_draws_with_the_preview() {
    let mut t = crate::testing::TestApp::new();
    let master = t.app.session.font().unwrap().masters[0].id.clone();
    t.app.session.execute("path.addRectangle", &json!({"glyph": "A", "layer": master, "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    t.app.view.font_view.selected = vec!["A".into()];
    t.ok(
        "engine.execute",
        json!({"command": "dialog.actions", "params": {"command": "filter.roundCorners", "set": {"radius": 30}, "preview": true}}),
    );
    for wireframe in [false, true] {
        t.ok("engine.execute", json!({"command": "dialog.actions", "params": {"wireframe": wireframe}}));
        t.frame();
        t.frame();
    }
    assert!(t.app.view.actions.shown.after.is_some());
    assert!(t.app.session.interaction().is_none(), "the preview leaves no interaction running");
    t.ok("engine.execute", json!({"command": "dialog.actions", "params": {"open": false}}));
    t.frame();
    assert_eq!(t.ok("ui.inspect", json!({}))["actions"]["open"], false);
    assert!(t.app.view.actions.shown.before.is_none());
}
