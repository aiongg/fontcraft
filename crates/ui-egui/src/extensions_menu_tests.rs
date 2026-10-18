//! Extensions in the interface: the Script menu mirrors the Macros folder with submenus and
//! tooltips, the Filter menu lists filter modules, View has a Show entry per reporter module
//! whose drawing the canvas paints, filter modules with parameters get a Palette section, and
//! the Font View selection becomes the `glyphs` of what these items run.

use fontcraft_engine::extensions::FolderStamps;
use fontcraft_engine::plugins::fixtures;
use serde_json::{Value, json};

use crate::FontcraftApp;
use crate::panels::palette_modules;
use crate::shell::menus::menu_tree_for;
use crate::shell::menus_dynamic::DynamicItem;

const SQUARE: &str = "org.test.square";
const SHIFT: &str = "org.test.shift";
const BOX: &str = "org.test.box";

/// The shapes of a 100 × 100 square at the origin, as a filter returns them.
fn square_shapes() -> Value {
    let mut app = FontcraftApp::default();
    let nodes = json!([{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}, {"x": 0, "y": 100}]);
    app.run("path.addPath", &json!({"glyph": "A", "nodes": nodes})).unwrap();
    let font = app.session.font().unwrap();
    let layer = font.glyph("A").unwrap().layers.iter().find(|l| l.is_master_layer()).unwrap();
    serde_json::to_value(&layer.shapes).unwrap()
}

/// Installs, turned on, a filter module `id` named `name` with `params` that turns every layer
/// into the square.
fn install_filter(app: &mut FontcraftApp, id: &str, name: &str, params: &str) {
    let manifest = format!(r#"{{"id":"{id}","name":"{name}","version":"1","kind":"filter","description":"Makes a square","params":{params}}}"#);
    let module = fixtures::returning(&manifest, &json!({"shapes": square_shapes()}).to_string());
    app.session.extensions_mut().registry.install_bytes(&module, None).unwrap();
}

/// Installs, turned on, a reporter module that returns `output`.
fn install_reporter(app: &mut FontcraftApp, id: &str, output: &Value) {
    let manifest = format!(r#"{{"id":"{id}","name":"Box","version":"1","kind":"reporter"}}"#);
    app.session.extensions_mut().registry.install_bytes(&fixtures::returning(&manifest, &output.to_string()), None).unwrap();
}

/// The dynamic items of the menu titled `title`.
fn dynamic(app: &FontcraftApp, title: &str) -> Vec<DynamicItem> {
    menu_tree_for(&app.session, &app.state).into_iter().find(|m| m.title == title).map(|m| m.dynamic).unwrap_or_default()
}

fn master_shapes(app: &FontcraftApp, glyph: &str) -> Value {
    let font = app.session.font().unwrap();
    let layer = font.glyph(glyph).unwrap().layers.iter().find(|l| l.is_master_layer()).unwrap();
    serde_json::to_value(&layer.shapes).unwrap()
}

fn undo_steps(app: &FontcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// The app editing `glyph` at actual size, with (300, 300) in the middle of the canvas.
fn editing(glyph: &str) -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": [glyph]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [300, 300]})).unwrap();
    app
}

/// The shapes of the last of `n` frames at 1280 × 800, nested lists flattened.
fn drawn(app: &mut FontcraftApp, n: usize) -> Vec<egui::Shape> {
    let ctx = egui::Context::default();
    let input = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = Vec::new();
    for _ in 0..n {
        let mut output = ctx.run_ui(input(), |ui| app.ui(ui));
        output.textures_delta.clear();
        out = output.shapes.into_iter().map(|c| c.shape).collect();
    }
    let mut flat = Vec::new();
    while let Some(shape) = out.pop() {
        match shape {
            egui::Shape::Vec(list) => out.extend(list),
            other => flat.push(other),
        }
    }
    flat
}

#[test]
fn script_menu_mirrors_the_macros_folder_with_submenus_tooltips_and_the_font_view_selection() {
    let mut app = FontcraftApp::default();
    let hide = json!({
        "format": "fontcraft.macro", "version": 1, "title": "Zeta", "description": "Leaves the glyphs out of exports",
        "steps": [{"command": "glyph.setExport", "params": {"glyphs": {"$arg": "glyphs"}, "export": false}}],
    });
    let plain = |title: &str| json!({"format": "fontcraft.macro", "version": 1, "title": title, "steps": []}).to_string().into_bytes();
    let untitled = json!({"format": "fontcraft.macro", "version": 1, "steps": []}).to_string().into_bytes();
    let files = vec![
        ("Zeta.fcmacro".to_string(), Ok(hide.to_string().into_bytes())),
        ("alpha.fcmacro".to_string(), Ok(untitled)),
        ("Tools/Inner.fcmacro".to_string(), Ok(plain("Inner"))),
        ("Tools/Deep/Deeper.fcmacro".to_string(), Ok(plain("Deeper"))),
        ("Broken.fcmacro".to_string(), Ok(b"not a macro".to_vec())),
    ];
    app.session.extensions_mut().sync_macros(files, FolderStamps::default());

    let items = dynamic(&app, "Script");
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Deeper", "Inner", "alpha", "Broken", "Zeta"], "folders first, then titles ignoring case");
    let paths: Vec<Vec<String>> = items.iter().map(|i| i.path.clone()).collect();
    assert_eq!(paths[0], ["Tools", "Deep"]);
    assert_eq!(paths[1], ["Tools"]);
    assert!(paths[2..].iter().all(Vec::is_empty));
    let zeta = &items[4];
    assert_eq!(zeta.tooltip.as_deref(), Some("Leaves the glyphs out of exports"));
    assert!(zeta.enabled);
    assert_eq!(items[2].tooltip, None, "no description, no tooltip");
    let broken = &items[3];
    assert!(!broken.enabled, "a file that cannot be read is greyed out");
    assert!(broken.tooltip.as_deref().is_some_and(|t| t.starts_with("This macro cannot be read")));

    // Font View's selection becomes the macro's `glyphs`.
    app.state.selected_glyphs = vec!["A".into(), "B".into()];
    let zeta = dynamic(&app, "Script").into_iter().find(|i| i.label == "Zeta").unwrap();
    assert_eq!(zeta.command, "script.run");
    assert_eq!(zeta.params, json!({"macro": "Zeta.fcmacro", "glyphs": ["A", "B"]}));
    app.run(zeta.command, &zeta.params).unwrap();
    let font = app.session.font().unwrap();
    assert!(!font.glyph("A").unwrap().export && !font.glyph("B").unwrap().export);
    assert!(font.glyph("C").unwrap().export, "glyphs outside the selection keep their setting");
}

#[test]
fn filter_menu_lists_filter_modules_and_runs_them_on_the_font_view_selection() {
    let mut app = FontcraftApp::default();
    assert!(dynamic(&app, "Filter").is_empty());
    install_filter(&mut app, SQUARE, "Square", "{}");

    // Nothing selected: listed, greyed out, with the reason.
    let items = dynamic(&app, "Filter");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].label, "Square");
    assert!(!items[0].enabled);

    app.state.selected_glyphs = vec!["A".into(), "B".into()];
    let item = dynamic(&app, "Filter").remove(0);
    assert!(item.enabled);
    assert_eq!(item.command, "plugin.runFilter");
    assert_eq!(item.tooltip.as_deref(), Some("Makes a square"));
    assert_eq!(item.params["id"], SQUARE);
    assert_eq!(item.params["glyphs"], json!(["A", "B"]));
    let master = app.session.font().unwrap().masters[0].id.clone();
    assert_eq!(item.params["layer"], json!(master), "Font View's master");

    let before = undo_steps(&app);
    app.run(item.command, &item.params).unwrap();
    assert_eq!(master_shapes(&app, "A"), square_shapes());
    assert_eq!(master_shapes(&app, "B"), square_shapes());
    assert_ne!(master_shapes(&app, "C"), square_shapes());
    assert_eq!(undo_steps(&app), before + 1, "one undo step for the whole selection");

    // A module turned off stays listed, greyed out with the reason.
    app.session.extensions_mut().set_module_enabled(SQUARE, false).unwrap();
    let item = dynamic(&app, "Filter").remove(0);
    assert!(!item.enabled);
    assert!(item.tooltip.as_deref().is_some_and(|t| t.contains("turned off")), "{:?}", item.tooltip);
}

#[test]
fn view_show_reporter_toggles_a_reporter_whose_primitives_are_drawn() {
    let mut app = editing("A");
    let fill = [250, 10, 20, 255];
    let line = [1, 2, 3, 255];
    install_reporter(
        &mut app,
        BOX,
        &json!({
            "background": [{"type": "line", "from": [0, 0], "to": [500, 0], "stroke": {"color": line, "width": 3}}],
            "foreground": [{"type": "circle", "center": [300, 300], "radius": 40, "fill": fill}],
        }),
    );
    let red = egui::Color32::from_rgba_unmultiplied(250, 10, 20, 255);
    let dark = egui::Color32::from_rgba_unmultiplied(1, 2, 3, 255);
    let circles = |shapes: &[egui::Shape]| shapes.iter().filter(|s| matches!(s, egui::Shape::Circle(c) if c.fill == red)).count();
    let lines =
        |shapes: &[egui::Shape]| shapes.iter().filter(|s| matches!(s, egui::Shape::LineSegment { stroke, .. } if stroke.color == dark)).count();

    let item = dynamic(&app, "View").into_iter().find(|i| i.command == "view.showReporter").unwrap();
    assert_eq!(item.label, "Show Box");
    assert_eq!(item.checked, Some(false));
    let shapes = drawn(&mut app, 2);
    assert_eq!(circles(&shapes), 0, "not drawn until shown");
    assert!(app.session.extensions().reporter_cache.is_empty(), "a hidden reporter does not run");

    let r = app.run(item.command, &item.params).unwrap();
    assert_eq!(r["on"], true);
    assert_eq!(app.state.panels.extensions.reporters, [BOX]);
    let item = dynamic(&app, "View").into_iter().find(|i| i.command == "view.showReporter").unwrap();
    assert_eq!(item.checked, Some(true), "the entry is checked while shown");
    let shapes = drawn(&mut app, 2);
    assert_eq!(circles(&shapes), 1, "the foreground circle");
    assert_eq!(lines(&shapes), 1, "the background line");
    assert_eq!(app.session.extensions().reporter_cache.len(), 1, "one run, cached for the layer and scale");

    app.run(item.command, &item.params).unwrap();
    assert!(app.state.panels.extensions.reporters.is_empty());
    assert_eq!(circles(&drawn(&mut app, 1)), 0);

    // Only installed reporters can be switched on; a failed switch changes nothing.
    assert!(app.run("view.showReporter", &json!({"id": "org.none", "on": true})).is_err());
    install_filter(&mut app, SQUARE, "Square", "{}");
    assert!(app.run("view.showReporter", &json!({"id": SQUARE, "on": true})).unwrap_err().contains("not a reporter"));
    assert!(app.state.panels.extensions.reporters.is_empty());
}

#[test]
fn a_failing_reporter_shows_its_error_and_is_not_run_every_frame() {
    let mut app = editing("A");
    install_reporter(&mut app, BOX, &json!({"error": "no stems found"}));
    app.run("view.showReporter", &json!({"id": BOX})).unwrap();
    let shapes = drawn(&mut app, 3);
    let shown = shapes.iter().any(|s| matches!(s, egui::Shape::Text(t) if t.galley.text().contains("no stems found")));
    assert!(shown, "the error is written on the canvas");
    assert_eq!(app.session.extensions().reporter_cache.len(), 1, "the failure is cached, not retried each frame");
}

#[test]
fn a_palette_section_applies_a_filter_module_with_its_form() {
    let mut app = editing("A");
    install_filter(&mut app, SQUARE, "Square", "{}");
    install_filter(&mut app, SHIFT, "Shift", r#"{"dx": {"type": "number", "min": -100, "max": 100, "default": 10}}"#);

    let sections = palette_modules::sections(&app.session);
    assert_eq!(sections.len(), 1, "only filter modules with parameters get a section");
    assert_eq!(sections[0].id, SHIFT);
    assert_eq!(sections[0].params.len(), 1);

    let mut values = serde_json::Map::new();
    values.insert("dx".into(), json!(25));
    let (command, params) = palette_modules::apply(&app.session, &app.state, SHIFT, &values).unwrap();
    assert_eq!(command, "plugin.runFilter");
    assert_eq!(params["glyph"], "A", "the edited glyph");
    assert!(params["layer"].is_string());
    assert_eq!(params["params"], json!({"dx": 25}));
    app.run(command, &params).unwrap();
    assert_eq!(master_shapes(&app, "A"), square_shapes());

    // The section collapses like the built-in ones; other names are refused.
    let r = app.run("palette.toggleSection", &json!({"section": SHIFT, "expanded": false})).unwrap();
    assert_eq!(r["expanded"], false);
    assert!(app.state.panels.collapsed.iter().any(|s| s == SHIFT));
    assert!(app.run("palette.toggleSection", &json!({"section": SQUARE})).is_err(), "a module without parameters has no section");
    app.run("palette.toggleSection", &json!({"section": SHIFT})).unwrap();
    assert!(!app.state.panels.collapsed.iter().any(|s| s == SHIFT));

    // The Palette draws the section and its form.
    app.state.show_palette = true;
    assert!(!drawn(&mut app, 2).is_empty());

    // A module turned off cannot be applied.
    app.session.extensions_mut().set_module_enabled(SHIFT, false).unwrap();
    assert!(palette_modules::apply(&app.session, &app.state, SHIFT, &values).is_err());
}
