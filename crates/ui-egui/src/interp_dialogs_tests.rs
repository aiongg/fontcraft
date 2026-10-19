//! The interpolation dialogs (M9.23): each one filled over the control channel gives the font its
//! command gives, and Cancel changes nothing.

use std::collections::BTreeMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Axis, Font, Glyph, GlyphTable, Instance, Layer, Master, Path, Shape};
use serde_json::{Value, json};

use crate::{FontcraftApp, control};

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
    assert_eq!(r["ok"], true, "{method} {params}: {r}");
    r["result"].clone()
}

fn refused(app: &mut FontcraftApp, method: &str, params: Value) -> String {
    let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
    assert_eq!(r["ok"], false, "{method} {params} should be refused: {r}");
    r["error"].as_str().unwrap_or_default().to_string()
}

fn execute(app: &mut FontcraftApp, command: &str, params: Value) -> Value {
    call(app, "engine.execute", json!({"command": command, "params": params}))
}

fn font_of(app: &FontcraftApp) -> Arc<Font> {
    app.session.doc().unwrap().font.clone()
}

fn undo_steps(app: &FontcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn rect(x0: f64, x1: f64) -> Shape {
    Shape::Path(Path::rect(x0, 0.0, x1, 700.0))
}

fn layer(master: &str, id: &str, width: f64, shapes: Vec<Shape>) -> Layer {
    let mut l = Layer::new(master, width);
    l.id = id.into();
    l.shapes = shapes;
    l
}

fn glyph(name: &str, layers: Vec<Layer>) -> Glyph {
    let mut g = Glyph::new(name);
    g.layers = layers;
    g
}

/// Masters Light (wght 30) and Bold (wght 120), both at wdth 100 when `width` adds that axis; the
/// instance Regular (wght 75) and the variable font setting VF. Glyphs: `n` (one rectangle per
/// master) and, with `width`, `o` (two rectangles per master and an intermediate layer of Light
/// at wght 75).
fn interp_font(name: &str, width: bool) -> Font {
    let mut f = Font::new(name);
    let mut axes = vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }];
    if width {
        axes.push(Axis { name: "Width".into(), tag: "wdth".into(), ..Axis::default() });
    }
    let at = |w: f64| if width { vec![w, 100.0] } else { vec![w] };
    f.axes = Arc::new(axes);
    let template = f.masters[0].clone();
    f.masters = Arc::new(vec![
        Master { id: "light".into(), name: "Light".into(), location: at(30.0), ..template.clone() },
        Master { id: "bold".into(), name: "Bold".into(), location: at(120.0), ..template },
    ]);
    f.instances = Arc::new(vec![
        Instance { name: "Regular".into(), location: at(75.0), ..Instance::default() },
        Instance { name: "VF".into(), kind: Some("variable".into()), ..Instance::default() },
    ]);
    let mut glyphs =
        vec![glyph("n", vec![layer("light", "light", 500.0, vec![rect(50.0, 150.0)]), layer("bold", "bold", 700.0, vec![rect(50.0, 350.0)])])];
    if width {
        let mut mid = layer("light", "o.mid", 600.0, vec![rect(50.0, 120.0), rect(220.0, 320.0)]);
        mid.info.attrs.coordinates = vec![Some(75.0), None];
        glyphs.push(glyph(
            "o",
            vec![
                layer("light", "light", 500.0, vec![rect(50.0, 100.0), rect(200.0, 300.0)]),
                layer("bold", "bold", 700.0, vec![rect(50.0, 150.0), rect(250.0, 400.0)]),
                mid,
            ],
        ));
    }
    f.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect());
    f
}

fn app_with(fonts: Vec<Font>) -> FontcraftApp {
    let mut s = Session::new();
    for f in fonts {
        s.add_document(f, None).unwrap();
    }
    let mut app = FontcraftApp::new(s);
    app.run("document.activate", &json!({"index": 0})).unwrap();
    app
}

/// Runs `n` frames of the app at 1280 × 800; the number of shapes the last one drew.
fn frames(app: &mut FontcraftApp, n: usize) -> usize {
    let ctx = egui::Context::default();
    let mut shapes = 0;
    for _ in 0..n {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
        shapes = output.shapes.len();
    }
    shapes
}

#[test]
fn the_axis_location_dialog_gives_the_font_its_command_gives() {
    let setup = || {
        let mut app = app_with(vec![interp_font("Axis Location", true)]);
        app.run("font.customParameters.add", &json!({"scope": "master", "targets": "bold", "name": "Axis Location"})).unwrap();
        app
    };
    let (mut app, mut direct) = (setup(), setup());

    let open = json!({"dialog": "dialog.axisLocation", "params": {"targets": "light"}});
    let error = refused(&mut app, "engine.execute", json!({"command": "dialog.open", "params": open}));
    assert!(error.contains("add it first"), "a master without the parameter has nothing to edit: {error}");
    let error = refused(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.axisLocation"}}));
    assert!(error.contains("targets"), "two masters: the dialog needs to know which: {error}");

    let opened = execute(&mut app, "dialog.open", json!({"dialog": "dialog.axisLocation", "params": {"targets": "bold"}}));
    assert_eq!(opened["dialog"], "dialog.axisLocation");
    assert_eq!(
        opened["params"]["value"],
        json!([{"Axis": "Weight", "Location": 120.0}, {"Axis": "Width", "Location": 100.0}]),
        "an empty value starts at the master's internal coordinates"
    );
    call(&mut app, "ui.dialog.set", json!({"field": "Weight", "value": 700}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(done["params"]["value"], json!([{"Axis": "Weight", "Location": 700.0}, {"Axis": "Width", "Location": 100.0}]));

    direct.run("font.customParameters.set", &done["params"]).unwrap();
    assert_eq!(font_of(&app).masters, font_of(&direct).masters, "the dialog gives the font the command gives");
    let bold = &font_of(&app).masters[1];
    let stored = fontcraft_engine::interp::params::axis_location(&bold.params).unwrap().unwrap();
    assert_eq!(stored.iter().map(|e| (e.axis.as_str(), e.location)).collect::<Vec<_>>(), [("Weight", 700.0), ("Width", 100.0)]);

    // Opened again, it shows the stored value; Cancel leaves the very font it opened on.
    let original = font_of(&app);
    let again = execute(&mut app, "dialog.open", json!({"dialog": "dialog.axisLocation", "params": {"targets": 1}}));
    assert_eq!(again["params"]["value"][0]["Location"], 700.0);
    call(&mut app, "ui.dialog.set", json!({"field": "Width", "value": 50}));
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert!(Arc::ptr_eq(&font_of(&app), &original));
}

#[test]
fn the_virtual_master_dialog_edits_one_parameter_row() {
    let setup = || {
        let mut app = app_with(vec![interp_font("Virtual Master", true)]);
        app.run("font.customParameters.add", &json!({"scope": "font", "name": "Virtual Master"})).unwrap();
        app
    };
    let (mut app, mut direct) = (setup(), setup());
    let opened = execute(&mut app, "dialog.open", json!({"dialog": "dialog.virtualMaster"}));
    assert_eq!(opened["params"]["scope"], "font");
    assert_eq!(
        opened["params"]["value"],
        json!([{"Axis": "Weight", "Location": 30.0}, {"Axis": "Width", "Location": 100.0}]),
        "an empty value starts at the first master"
    );
    call(&mut app, "ui.dialog.set", json!({"field": "Width", "value": 200}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    direct.run("font.customParameters.set", &done["params"]).unwrap();
    assert_eq!(font_of(&app).info.params, font_of(&direct).info.params);
    let stored = fontcraft_engine::interp::params::virtual_masters(&font_of(&app)).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].coordinates, [("Weight".to_string(), 30.0), ("Width".to_string(), 200.0)]);

    // With two rows the dialog needs `index`; Cancel changes nothing.
    app.run("font.customParameters.add", &json!({"scope": "font", "name": "Virtual Master"})).unwrap();
    let error = refused(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.virtualMaster"}}));
    assert!(error.contains("index"), "{error}");
    let original = font_of(&app);
    let second = execute(&mut app, "dialog.open", json!({"dialog": "dialog.virtualMaster", "params": {"index": 1}}));
    assert_eq!(second["params"]["index"], 1);
    call(&mut app, "ui.dialog.set", json!({"field": "Weight", "value": 400}));
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert!(Arc::ptr_eq(&font_of(&app), &original));
}

#[test]
fn add_other_font_copies_the_ticked_masters_as_its_command_does() {
    let other = || {
        let mut f = interp_font("Other", false);
        Arc::make_mut(&mut f.masters).truncate(1);
        let black = Arc::make_mut(&mut f.masters).first_mut().unwrap();
        (black.id, black.name, black.location) = ("black".into(), "Black".into(), vec![200.0]);
        f.glyphs = GlyphTable::from_glyphs(vec![Arc::new(glyph("n", vec![layer("black", "black", 800.0, vec![rect(50.0, 450.0)])]))]);
        f
    };
    let (mut app, mut direct) = (app_with(vec![interp_font("Main", false), other()]), app_with(vec![interp_font("Main", false), other()]));
    let opened = execute(&mut app, "dialog.open", json!({"dialog": "fontInfo.addOtherFont"}));
    assert_eq!(opened["fields"][0]["id"], "sources");
    assert_eq!(opened["fields"][0]["choices"], json!(["0/0", "0/1", "1/0"]), "every master of every open font");
    assert_eq!(opened["fields"][0]["labels"][2], "Untitled 2 — Black", "document title, then master name");

    // Cancel adds nothing.
    let original = font_of(&app);
    call(&mut app, "ui.dialog.set", json!({"field": "sources", "value": ["1/0"]}));
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert!(Arc::ptr_eq(&font_of(&app), &original));

    let steps = undo_steps(&app);
    execute(&mut app, "dialog.open", json!({"dialog": "fontInfo.addOtherFont"}));
    call(&mut app, "ui.dialog.set", json!({"field": "sources", "value": ["1/0"]}));
    let done = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(done["params"]["sources"], json!([{"font": 1, "master": 0}]));
    assert_eq!(undo_steps(&app), steps + 1, "one undo step");

    direct.run("font.master.addFromOtherFont", &json!({"sources": [{"font": 1, "master": 0}]})).unwrap();
    let (a, b) = (font_of(&app), font_of(&direct));
    let names = |f: &Font| f.masters.iter().map(|m| (m.name.clone(), m.location.clone())).collect::<Vec<_>>();
    assert_eq!(names(&a), names(&b));
    assert_eq!(names(&a).last(), Some(&("Black".to_string(), vec![200.0])));
    assert_eq!(a.glyph("n").unwrap().layers.len(), b.glyph("n").unwrap().layers.len());
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-interp-dialogs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn files(dir: &FsPath) -> BTreeMap<String, Vec<u8>> {
    let Ok(entries) = std::fs::read_dir(dir) else { return BTreeMap::new() };
    entries.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap())).collect()
}

#[test]
fn the_variable_fonts_tab_exports_what_its_command_exports() {
    let (by_dialog, by_command) = (temp("dialog"), temp("command"));
    let mut app = app_with(vec![interp_font("Variable Tab", false)]);
    let opened = execute(&mut app, "file.export", json!({"tab": "variable", "params": {"destination": by_dialog, "timestamp": 0}}));
    assert_eq!(opened["dialog"], "dialog.exportVariable");
    assert_eq!(opened["command"], "file.exportVariable");
    let ids: Vec<&str> = opened["fields"].as_array().unwrap().iter().filter_map(|f| f["id"].as_str()).collect();
    assert_eq!(ids, ["containers", "destination"], "the OTF tab's formats and folder, and no flavour");
    assert_eq!(opened["tabs"][1], json!({"id": "dialog.exportVariable", "tab": "variable", "label": "Variable Fonts", "active": true}));

    call(&mut app, "ui.dialog.set", json!({"field": "containers", "value": ["plain", "woff2"]}));
    assert!(files(&by_dialog).is_empty(), "nothing is written before Next…");
    call(&mut app, "ui.dialog.confirm", json!({}));

    let mut direct = app_with(vec![interp_font("Variable Tab", false)]);
    direct.run("file.exportVariable", &json!({"containers": ["plain", "woff2"], "destination": by_command, "timestamp": 0})).unwrap();
    let (wrote, expected) = (files(&by_dialog), files(&by_command));
    assert_eq!(expected.len(), 2, "{:?}", expected.keys());
    assert_eq!(wrote, expected);
    let _ = std::fs::remove_dir_all(&by_dialog);
    let _ = std::fs::remove_dir_all(&by_command);
}

#[test]
fn the_axis_mappings_dialog_previews_its_edits_and_ok_keeps_one_undo_step() {
    // wght maps 30 → 30 and 120 → 120; wdth has no entry.
    let mapped = || {
        let mut app = app_with(vec![interp_font("Axis Mappings", true)]);
        app.run("font.customParameters.add", &json!({"scope": "font", "name": "Axis Mappings", "value": {"wght": {"30": 30, "120": 120}}})).unwrap();
        app
    };
    let mut app = mapped();
    let steps = undo_steps(&app);
    let shown = execute(&mut app, "axisMappings.showDialog", json!({}));
    assert_eq!((shown["open"].clone(), shown["axis"].clone()), (json!(true), json!("wght")));
    assert_eq!(shown["axes"].as_array().unwrap().len(), 2);
    assert_eq!(shown["axes"][0]["points"], json!([{"internal": 30.0, "external": 30.0}, {"internal": 120.0, "external": 120.0}]));
    frames(&mut app, 2);

    // While the dialog is open, other edits wait.
    assert!(app.run("glyph.newGlyph", &json!({"name": "x"})).is_err());

    let added = execute(&mut app, "axisMappings.dialogEdit", json!({"command": "axisMappings.addPoint", "params": {"internal": 75}}));
    assert_eq!(added["axes"][0]["points"][1], json!({"internal": 75.0, "external": 75.0}), "the preview shows the new point, on the line");
    assert_eq!(added["point"], 1, "the new point is selected");
    execute(&mut app, "axisMappings.dialogEdit", json!({"command": "axisMappings.nudgePoint", "params": {"dExternal": 10}}));
    let nudged = execute(&mut app, "axisMappings.dialogEdit", json!({"command": "axisMappings.nudgePoint", "params": {"dExternal": 5}}));
    assert_eq!(nudged["edits"], 2, "moves of one point merge into one step");

    // An edit that breaks the order changes nothing.
    let error = refused(
        &mut app,
        "engine.execute",
        json!({"command": "axisMappings.dialogEdit", "params": {"command": "axisMappings.setPoint", "params": {"external": 1_000_000}}}),
    );
    assert!(!error.is_empty());
    assert_eq!(execute(&mut app, "axisMappings.dialogEdit", json!({})), nudged, "the refused edit left the dialog as it was");
    let bad = json!({"command": "axisMappings.dialogEdit", "params": {"axis": "wdth", "command": "axisMappings.nope"}});
    assert!(!refused(&mut app, "engine.execute", bad).is_empty());
    assert_eq!(execute(&mut app, "axisMappings.dialogEdit", json!({})), nudged, "a refused request keeps the axis it was on");
    assert_eq!(execute(&mut app, "axisMappings.dialogEdit", json!({"point": null}))["point"], json!(null), "null clears the selection");
    frames(&mut app, 2);

    let done = execute(&mut app, "axisMappings.closeDialog", json!({"confirm": true}));
    assert_eq!(done, json!({"confirmed": true, "changed": true}));
    assert_eq!(undo_steps(&app), steps + 1, "OK makes one undo step");
    assert!(!app.state.panels.axis_mappings.open);

    let mut direct = mapped();
    direct.run("axisMappings.addPoint", &json!({"axis": "wght", "internal": 75})).unwrap();
    direct.run("axisMappings.nudgePoint", &json!({"axis": "wght", "index": 1, "dExternal": 15})).unwrap();
    assert_eq!(font_of(&app).info.params, font_of(&direct).info.params, "the dialog gives the font its commands give");

    // Cancel gives back the very font the dialog opened on, also for a variable font setting.
    let original = font_of(&app);
    execute(&mut app, "axisMappings.showDialog", json!({"variableFontSetting": "VF", "axis": "wdth"}));
    execute(&mut app, "axisMappings.dialogEdit", json!({"command": "axisMappings.addPoint", "params": {"internal": 110, "external": 110}}));
    assert_eq!(execute(&mut app, "axisMappings.closeDialog", json!({})), json!({"confirmed": false, "changed": false}));
    assert!(Arc::ptr_eq(&font_of(&app), &original));
    assert!(
        refused(&mut app, "engine.execute", json!({"command": "axisMappings.showDialog", "params": {"variableFontSetting": "Regular"}}))
            .contains("static")
    );
    assert!(app.session.interaction().is_none(), "a refused opening begins nothing");
}

#[test]
fn the_shape_order_window_lists_every_interpolating_layer_and_reorders_one() {
    let mut app = app_with(vec![interp_font("Shape Order", true)]);
    let shown = execute(&mut app, "window.shapeOrder", json!({"glyph": "o"}));
    assert_eq!(shown["open"], true);
    let layers: Vec<&str> = shown["layers"].as_array().unwrap().iter().filter_map(|l| l["layer"].as_str()).collect();
    assert_eq!(layers, ["light", "o.mid", "bold"], "each master, followed by its special layers");
    assert_eq!(shown["layers"][0]["name"], "Light");
    assert!(shown["layers"].as_array().unwrap().iter().all(|l| l["shapes"].as_array().unwrap().len() == 2));
    assert!(frames(&mut app, 2) > 0);

    // A drop of shape 1 on shape 0 of the intermediate layer runs this reorder.
    let order = crate::dialogs::form::moved(&[0, 1], 1, 0).unwrap();
    let before = font_of(&app).glyph("o").unwrap().layers[2].shapes.clone();
    app.run("glyph.setShapeOrder", &json!({"glyph": "o", "layer": "o.mid", "order": order})).unwrap();
    let after = font_of(&app).glyph("o").unwrap().layers[2].shapes.clone();
    assert_eq!(after, [before[1].clone(), before[0].clone()]);

    // A refused request leaves the window as it was.
    assert!(app.run("window.shapeOrder", &json!({"glyph": "missing"})).is_err());
    assert_eq!(app.state.panels.shape_order_window.glyph.as_deref(), Some("o"));
    assert_eq!(execute(&mut app, "window.shapeOrder", json!({"open": false}))["open"], false);
}
