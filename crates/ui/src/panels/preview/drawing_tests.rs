//! What the strip draws: the line shaped with the tab's features, an instance through its own preview font, and colour
//! glyphs, inverted and turned with the line.

use std::sync::Arc;
use std::time::{Duration, Instant};

use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_model::{Axis, Glyph, GlyphTable, Instance, Layer, Path, Shape};
use serde_json::{Value, json};

use super::*;
use crate::app::models::preview::shown_line;
use crate::app::{APP, FontCraftApp, ViewMsg};

fn post(app: &mut FontCraftApp, msg: Msg) -> Value {
    app.run_now(msg, false).unwrap_or_else(|e| panic!("{e}"))
}

fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
    post(app, Msg::Shell(CommandRef::with_params(id, params)));
    app.project();
}

fn run(app: &mut FontCraftApp, id: &str, params: Value) -> Value {
    let reply = post(app, Msg::run(id, params));
    app.project();
    reply
}

/// Masters Thin (100) and Black (900), an instance Light (300) and one Regular (500); `a`, `b` and `a_b`, each a box;
/// the text "ab" with `a` edited and the Preview Area open.
fn app() -> FontCraftApp {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.session
        .edit("setup", |font| {
            font.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
            let mut thin = font.default_master()?.clone();
            thin.id = "thin".into();
            thin.name = "Thin".into();
            thin.location = vec![100.0];
            let mut black = thin.clone();
            black.id = "black".into();
            black.name = "Black".into();
            black.location = vec![900.0];
            font.masters = Arc::new(vec![thin, black]);
            let instance = |name: &str, at: f64| Instance { name: name.into(), location: vec![at], ..Instance::default() };
            font.instances = Arc::new(vec![instance("Light", 300.0), instance("Regular", 500.0)]);
            let glyph = |name: &str, unicode: Option<u32>, width: f64| {
                let mut g = Glyph::new(name);
                g.unicodes = unicode.into_iter().collect();
                g.layers = ["thin", "black"]
                    .map(|m| {
                        let mut l = Layer::new(m, width);
                        l.shapes = vec![Shape::Path(Path::rect(0.0, 0.0, width - 100.0, 700.0))];
                        l
                    })
                    .to_vec();
                Arc::new(g)
            };
            font.glyphs = GlyphTable::from_glyphs(vec![glyph("a", Some(0x61), 400.0), glyph("b", Some(0x62), 500.0), glyph("a_b", None, 650.0)]);
            Ok(())
        })
        .unwrap();
    post(&mut app, Msg::View(ViewMsg::SelectGlyph("a".into())));
    shell(&mut app, "editView.setText", json!({"glyphs": ["a", "b"]}));
    shell(&mut app, "editView.setCursor", json!({"index": 0}));
    shell(&mut app, "view.togglePreviewArea", json!({"on": true}));
    app
}

fn advances(app: &FontCraftApp) -> Vec<f64> {
    app.models.preview_area.hits.iter().map(|h| h.advance).collect()
}

#[test]
fn the_line_is_shaped_with_the_tabs_features_and_an_instance_with_its_own() {
    let mut app = app();
    run(&mut app, "font.features.addFeature", json!({"tag": "liga", "code": "sub a b by a_b;"}));
    let compiled = run(&mut app, "font.features.compile", json!({}));
    assert_eq!(compiled["preview"], json!(true), "{compiled}");
    shell(&mut app, "window.previewPanel", json!({"on": true}));
    assert_eq!(advances(&app), [400.0, 500.0], "no feature is on");

    shell(&mut app, "editView.toggleFeature", json!({"tag": "liga"}));
    assert_eq!(advances(&app), [650.0], "the ligature");
    assert_eq!(app.models.preview_panel.hits.len(), 1, "in the panel too");
    let line = shown_line(&app.session, &app.view).expect("a line is shown");
    assert_eq!((line.glyphs.as_slice(), line.features.as_slice()), (&["a".to_string(), "b".into()][..], &["liga".to_string()][..]));

    // An instance whose parameter replaces the feature: shaped with the document's font until its own is built.
    run(
        &mut app,
        "font.customParameters.add",
        json!({"scope": "instance", "targets": ["Light"], "name": "Replace Feature", "value": "liga; sub b a by a_b;"}),
    );
    shell(&mut app, "preview.setInstance", json!({"instance": "Regular"}));
    assert_eq!(advances(&app), [650.0]);
    shell(&mut app, "preview.setInstance", json!({"instance": "Light"}));
    let start = Instant::now();
    while advances(&app) != [400.0, 500.0] && start.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(5));
        app.project();
    }
    assert_eq!(advances(&app), [400.0, 500.0], "Light has no such ligature");
}

/// Adds a full-colour layer of `master` to `a` with a red square and a blue one; its id.
fn two_colours(app: &mut FontCraftApp, master: &str) -> String {
    run(app, "layers.addBackupLayer", json!({"glyphs": ["a"], "master": master}));
    let layers = &app.session.font().unwrap().glyph("a").unwrap().layers;
    let layer = layers.iter().find(|l| !l.is_master_layer() && l.master == master).unwrap().id.clone();
    run(app, "layer.convertToColor", json!({"glyph": "a", "layer": layer}));
    for (i, (x, color)) in [(0, "#FF0000FF"), (200, "#0000FFFF")].into_iter().enumerate() {
        run(app, "path.addRectangle", json!({"glyph": "a", "layer": layer, "x": x, "y": 0, "width": 200, "height": 400}));
        run(app, "attributes.setFillColor", json!({"glyph": "a", "layer": layer, "shapes": [i + 1], "color": color}));
    }
    layer
}

/// Whether the strip shows the colour somewhere.
fn shows(app: &FontCraftApp, color: Color32) -> bool {
    let model: &PreviewModel = &app.models.preview_area;
    let tokens = PreviewTokens::standard();
    let (ink, paper) = if model.options.invert { (tokens.paper, tokens.ink) } else { (tokens.ink, tokens.paper) };
    let placement = place(model, [0.0, 0.0, 400.0, 100.0], 12.0).unwrap();
    picture(model, &placement, 1.0, ink, paper).unwrap().pixels.contains(&color)
}

#[test]
fn colour_glyphs_draw_in_the_strip_inverted_turned_and_for_all_instances() {
    let mut app = app();
    let layer = two_colours(&mut app, "thin");
    two_colours(&mut app, "black");
    let (red, blue) = (Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 0, 255));
    assert!(app.models.preview_area.colors.is_empty() && !shows(&app, red), "the master layer is shown");

    app.view.layers.layers.insert("a".into(), layer);
    app.view_changed();
    app.project();
    assert_eq!(app.models.preview_area.colors.len(), 1);
    assert!(!app.models.preview_area.outline.elements().is_empty(), "`b` has no colour layers: it draws in ink");
    for id in ["preview.setOffset", "preview.invert", "preview.flip"] {
        shell(&mut app, id, if id == "preview.setOffset" { json!({"x": 0}) } else { json!({"on": true}) });
        assert!(shows(&app, red) && shows(&app, blue), "{id}");
    }

    shell(&mut app, "preview.setInstance", json!({"instance": "Light"}));
    assert_eq!(app.models.preview_area.colors.len(), 1, "the instance's colour layers");
    shell(&mut app, "preview.showAllInstances", json!({}));
    assert_eq!(app.models.preview_area.colors.len(), 2, "one per instance");
    assert!(shows(&app, red) && shows(&app, blue));
}
