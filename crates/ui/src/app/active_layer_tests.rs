//! The shown layer and master outside the canvas tools: Font View's master, the glyph menu's layer colour, the clipboard
//! and the glyph panels follow `view.selectMaster` and the Layers panel's pick, and only a person's copy reaches the
//! system clipboard.

use std::sync::Arc;

use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::model::{ColorLabel, Layer};
use serde_json::{Value, json};

use super::context_menu::glyph_menu::{self, GlyphAction};
use super::{APP, FontCraftApp, Msg, ViewMsg, clipboard};
use crate::control;

fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
    app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
    app.drain();
    assert!(app.status.is_empty(), "{id}: {}", app.status);
}

/// Glyph A edited, with a square; a second master `m02` with a layer in every glyph, and a backup layer `bk` of the
/// first master in A.
fn two_master_app() -> FontCraftApp {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    for name in ["A", "B"] {
        app.session.execute("glyph.newGlyph", &json!({ "name": name })).ok();
    }
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    app.session.execute("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.session
        .edit("setup", |font| {
            let first = font.masters[0].id.clone();
            let mut m = font.default_master().unwrap().clone();
            m.id = "m02".into();
            m.name = "Bold".into();
            Arc::make_mut(&mut font.masters).push(m);
            let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
            for name in names {
                font.glyph_mut(&name)?.layers.push(Layer::new("m02", 600.0));
            }
            let mut backup = font.glyph("A").unwrap().layer(&first).unwrap().clone();
            backup.id = "bk".into();
            font.glyph_mut("A")?.layers.push(backup);
            Ok(())
        })
        .unwrap();
    app.drain();
    app
}

fn first_master(app: &FontCraftApp) -> String {
    app.session.font().unwrap().masters[0].id.clone()
}

#[test]
fn font_view_items_follow_the_current_master() {
    let mut app = two_master_app();
    shell(&mut app, "fontView.show", json!({}));
    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["A", "B"]}));
    shell(&mut app, "view.selectMaster", json!({"master": "m02"}));

    assert_eq!(app.models.font_view.master.as_deref(), Some("m02"), "the cells draw the current master");
    let filled = app.with_font_view_targets(CommandRef::new("glyph.setLayerColorLabel"));
    assert_eq!(filled.params["layer"], json!("m02"), "{}", filled.params);

    // The glyph menu's Alt swatches colour the current master's layers of its glyphs.
    let names = vec!["A".to_string(), "B".to_string()];
    let menu = glyph_menu::open(&app, &names, [0.0, 0.0]);
    assert_eq!(menu.layer.as_deref(), Some("m02"));
    for msg in glyph_menu::steps(&menu, &GlyphAction::LayerColor(Some(3))) {
        app.run_now(msg, false).unwrap();
    }
    let font = app.session.font().unwrap();
    for name in ["A", "B"] {
        let glyph = font.glyph(name).unwrap();
        assert_eq!(glyph.layer("m02").unwrap().info.color, Some(ColorLabel::Index(3)), "{name}'s m02 layer took the colour");
        assert_eq!(glyph.layer(&first_master(&app)).unwrap().info.color, None, "{name}'s first layer kept none");
    }

    // Copy in Font View copies the current master's layers.
    let copy = clipboard::fired(&mut app, CommandRef::new("edit.copy")).unwrap();
    assert_eq!(copy.params, json!({"glyphs": ["A", "B"], "master": "m02"}));
}

#[test]
fn one_glyph_menu_and_paste_special_follow_the_layer_the_layers_panel_picked() {
    let mut app = two_master_app();
    shell(&mut app, "layers.select", json!({"layer": "bk"}));
    let menu = glyph_menu::open(&app, &["A".to_string()], [0.0, 0.0]);
    assert_eq!(menu.layer.as_deref(), Some("bk"), "one glyph: the layer it shows");
    let menu = glyph_menu::open(&app, &["A".to_string(), "B".to_string()], [0.0, 0.0]);
    assert_eq!(menu.layer.as_deref(), Some(first_master(&app).as_str()), "several glyphs: the current master");

    // Paste Special on the canvas asks with the master of the shown layer, not the current master.
    shell(&mut app, "view.selectMaster", json!({"master": "m02"}));
    shell(&mut app, "layers.select", json!({"layer": "bk"}));
    let text = app.session.execute("edit.copy", &json!({"glyphs": ["B"]})).unwrap()["json"].as_str().unwrap().to_string();
    let Msg::Shell(open) = clipboard::paste_msg(&mut app, &text, clipboard::PasteKind::Special).unwrap() else {
        panic!("Paste Special opens a dialog")
    };
    assert_eq!(open.params["params"]["master"], json!(first_master(&app)), "{}", open.params);
}

#[test]
fn a_drawing_pastes_into_the_shown_layer() {
    let mut app = two_master_app();
    shell(&mut app, "view.selectMaster", json!({"master": "m02"}));
    let first = first_master(&app);
    let text = app.session.execute("edit.copy", &json!({"glyph": "A", "layer": first})).unwrap()["json"].as_str().unwrap().to_string();
    let msg = clipboard::paste_msg(&mut app, &text, clipboard::PasteKind::Paste).unwrap();
    app.run_now(msg, false).unwrap();
    let font = app.session.font().unwrap();
    let glyph = font.glyph("A").unwrap();
    assert_eq!(glyph.layer("m02").unwrap().shapes.len(), 1, "the square landed in the shown master's layer");
    assert_eq!(glyph.layer(&first).unwrap().shapes.len(), 1, "the first master's layer is unchanged");
}

#[test]
fn the_glyph_panels_read_the_shown_layer_and_master() {
    let mut app = two_master_app();
    assert_eq!(app.models.measurements.master.as_deref(), Some(first_master(&app).as_str()));
    shell(&mut app, "view.selectMaster", json!({"master": "m02"}));
    assert_eq!(app.models.measurements.master.as_deref(), Some("m02"));
    assert_eq!(app.models.selections.layer.as_deref(), Some("m02"));
    shell(&mut app, "layers.select", json!({"layer": "bk"}));
    assert_eq!(app.models.selections.layer.as_deref(), Some("bk"), "the stored selections of the picked layer");
    assert_eq!(app.models.measurements.master.as_deref(), Some(first_master(&app).as_str()), "the picked layer's master");
}

/// The Metrics panel reads and edits the layer the shown glyph shows in the current master.
#[test]
fn the_metrics_panel_follows_the_current_master() {
    let mut app = two_master_app();
    let first = first_master(&app);
    assert_eq!(app.models.glyph_metrics.layer.as_deref(), Some(first.as_str()));
    app.session.execute("spacing.setWidth", &json!({"glyphs": ["A"], "layer": "m02", "value": 640})).unwrap();
    app.drain();
    assert_ne!(app.models.glyph_metrics.width, Some(640.0), "the first master's width shows");
    shell(&mut app, "view.selectMaster", json!({"master": "m02"}));
    assert_eq!(app.models.glyph_metrics.layer.as_deref(), Some("m02"));
    assert_eq!(app.models.glyph_metrics.width, Some(640.0));
}

/// `edit.copy` named over the control channel returns its text and leaves the system clipboard alone; the same copy
/// from a person (a menu item, or the control channel playing one) puts the text there.
#[test]
fn only_a_persons_copy_reaches_the_system_clipboard() {
    let mut app = two_master_app();
    let reply = control::dispatch(&mut app, "engine.execute", &json!({"command": "edit.copy", "params": {"glyph": "A"}})).unwrap();
    let text = reply["json"].as_str().expect("the copied text in the reply").to_string();
    assert!(!text.is_empty());
    assert_eq!(app.clipboard.out, None, "a request does not write the clipboard");
    control::dispatch(
        &mut app,
        "engine.execute",
        &json!({"command": "glyph.copyGlyphNames", "params": {"glyphs": ["A"], "format": "spaceSeparated"}}),
    )
    .unwrap();
    assert_eq!(app.clipboard.out, None);

    app.run_now(Msg::run("edit.copy", json!({"glyph": "A"})), false).unwrap();
    assert_eq!(app.clipboard.out.as_deref(), Some(text.as_str()), "a person's copy does");
}
