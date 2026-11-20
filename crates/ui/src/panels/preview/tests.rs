//! The previews: the commands and the lines they project (the master, an instance, all instances), the placement
//! of the line in the strip, and the view's controls, double-click and drag.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::behavior::dock::PanelState;
use craft::host::Host;
use craft::ui::contract::Tracked;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use egui::Vec2;
use fontcraft_engine::Session;
use fontcraft_model::{Axis, Glyph, GlyphTable, Instance, Layer, Path, Shape};
use serde_json::{Value, json};

use super::*;
use crate::app::models::preview::{HitItem, InstanceEntry};
use crate::app::{APP, FontCraftApp, ViewMsg};
use crate::shell::dock::PanelId;
use crate::shell::preview::{PreviewOptions, Shows};

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

fn run(app: &mut FontCraftApp, msg: Msg) -> Result<Value, String> {
    app.run_now(msg, false)
}

/// A box `w` wide, advance `w + 100`.
fn layer(master: &str, w: f64) -> Layer {
    let mut l = Layer::new(master, w + 100.0);
    l.shapes = vec![Shape::Path(Path::rect(0.0, 0.0, w, 700.0))];
    l
}

/// Masters Thin (100) and Black (900); instances Light (300), Regular (500) and a variable font setting; `a` is 100
/// to 300 wide, `b` 200 to 600. The edit view holds "ab" with `a` edited, and the Preview Area is open.
fn instance_app() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
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
            font.instances = Arc::new(vec![
                Instance { name: "Light".into(), location: vec![300.0], ..Instance::default() },
                Instance { name: "Regular".into(), location: vec![500.0], ..Instance::default() },
                Instance { name: "Variable".into(), kind: Some("variable".into()), ..Instance::default() },
            ]);
            let glyph = |name: &str, unicode: u32, thin: f64, black: f64| {
                let mut g = Glyph::new(name);
                g.unicodes = vec![unicode];
                g.layers = vec![layer("thin", thin), layer("black", black)];
                Arc::new(g)
            };
            font.glyphs = GlyphTable::from_glyphs(vec![glyph("a", 0x61, 100.0, 300.0), glyph("b", 0x62, 200.0, 600.0)]);
            Ok(())
        })
        .unwrap();
    run(&mut app, Msg::View(ViewMsg::SelectGlyph("a".into()))).unwrap();
    run(&mut app, shell("editView.setText", json!({"glyphs": ["a", "b"]}))).unwrap();
    run(&mut app, shell("editView.setCursor", json!({"index": 0}))).unwrap();
    run(&mut app, shell("view.togglePreviewArea", json!({"on": true}))).unwrap();
    app
}

fn advances(model: &PreviewModel) -> Vec<f64> {
    model.hits.iter().map(|h| h.advance).collect()
}

#[test]
fn set_instance_lays_the_line_out_at_the_instance_and_the_panel_keeps_its_own_choice() {
    let mut app = instance_app();
    let area = &app.models.preview_area;
    assert!(area.shown && area.master_chosen);
    assert_eq!(advances(area), [200.0, 300.0], "the dash draws the current master");
    assert_eq!(area.extent, [0.0, 500.0]);
    assert_eq!(area.instances.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Light", "Regular"], "static instances only");

    run(&mut app, shell("preview.setInstance", json!({"instance": "Regular"}))).unwrap();
    let area = &app.models.preview_area;
    assert_eq!(area.instance_label.as_deref(), Some("Regular"));
    assert_eq!(advances(area), [300.0, 500.0], "the instance's widths");
    run(&mut app, shell("preview.setInstance", json!({"instance": 0}))).unwrap();
    assert_eq!(app.view.preview.area_shows, Shows::Instance("Light".into()));

    // The dash follows a master switch.
    run(&mut app, shell("preview.setInstance", json!({"instance": "master"}))).unwrap();
    run(&mut app, shell("view.selectMaster", json!({"master": 1}))).unwrap();
    assert_eq!(advances(&app.models.preview_area), [400.0, 700.0]);

    for bad in [json!({"instance": "Variable"}), json!({"instance": 7}), json!({"instance": "Nope"}), json!({}), json!({"instance": true})] {
        let before = app.view.preview.clone();
        assert!(run(&mut app, shell("preview.setInstance", bad.clone())).is_err(), "{bad}");
        assert_eq!(app.view.preview, before, "a refused choice changes nothing");
    }

    // The panel is blank while hidden and keeps a choice of its own.
    assert!(!app.models.preview_panel.shown);
    run(&mut app, shell("window.previewPanel", json!({}))).unwrap();
    assert_ne!(app.view.workspace.layout.panel_state(&PanelId::Preview), PanelState::Hidden);
    run(&mut app, shell("preview.setInstance", json!({"target": "panel", "instance": "Light"}))).unwrap();
    assert_eq!(app.models.preview_panel.instance_label.as_deref(), Some("Light"));
    assert!(app.models.preview_area.master_chosen);
    run(&mut app, shell("window.previewPanel", json!({"on": false}))).unwrap();
    assert!(!app.models.preview_panel.shown);
}

#[test]
fn show_all_instances_draws_the_edited_glyph_once_per_visible_instance() {
    let mut app = instance_app();
    run(&mut app, shell("preview.showAllInstances", json!({}))).unwrap();
    // Light 250 and Regular 300 wide; the variable font setting is not an instance here.
    let area = &app.models.preview_area;
    assert!(area.all_chosen);
    assert_eq!(area.extent, [0.0, 550.0]);
    assert!(area.hits.is_empty(), "a double-click edits nothing in the line of instances");

    run(&mut app, Msg::run("preview.toggleInstanceVisibility", json!({"instance": "Light"}))).unwrap();
    assert_eq!(app.models.preview_area.extent, [0.0, 300.0], "the hidden instance is left out");
    assert!(!app.models.preview_area.instances.iter().any(|i| i.name == "Light" && i.visible));
    run(&mut app, Msg::run("edit.undo", json!({}))).unwrap();
    assert_eq!(app.models.preview_area.extent, [0.0, 550.0]);
}

#[test]
fn the_area_options_take_targets_and_clamp_their_values() {
    let mut app = instance_app();
    run(&mut app, shell("preview.invert", json!({}))).unwrap();
    run(&mut app, shell("preview.setBlur", json!({"target": "panel", "amount": 99}))).unwrap();
    run(&mut app, shell("preview.setOffset", json!({"dx": 20}))).unwrap();
    run(&mut app, shell("preview.setOffset", json!({"dx": 5}))).unwrap();
    run(&mut app, shell("view.togglePreviewArea", json!({"on": true, "height": 1}))).unwrap();
    let p = &app.view.preview;
    assert!(p.area.invert && !p.panel.invert);
    assert_eq!((p.panel.blur, p.area.blur, p.area.offset), (10.0, 0.0, 25.0));
    assert_eq!(p.area_height, Some(PreviewTokens::standard().area_min_height));
    assert!(app.models.preview_area.options.invert, "the model carries the options");
    assert!(run(&mut app, shell("preview.setOffset", json!({"x": 1, "dx": 1}))).is_err());
    assert!(run(&mut app, shell("preview.flip", json!({"target": "elsewhere"}))).is_err());
    run(&mut app, shell("view.togglePreviewArea", json!({}))).unwrap();
    assert!(!app.view.preview.area_open && !app.models.preview_area.shown);
}

fn line_model(options: PreviewOptions) -> PreviewModel {
    PreviewModel {
        shown: true,
        key: 1,
        outline: fontcraft_geom::BezPath::new(),
        extent: [0.0, 1000.0],
        focus: Some(250.0),
        rtl: false,
        range: [-200.0, 800.0],
        hits: vec![HitItem { item: 0, x: 0.0, advance: 500.0 }, HitItem { item: 1, x: 500.0, advance: 500.0 }],
        options,
        instances: vec![InstanceEntry { index: 0, name: "Light".into(), visible: true, chosen: false }],
        master_chosen: true,
        all_chosen: false,
        instance_label: None,
    }
}

#[test]
fn the_line_is_centred_on_the_edited_glyph_or_kept_flush_and_flip_turns_it() {
    let rect = [0.0, 0.0, 2000.0, 100.0];
    let centred = place(&line_model(PreviewOptions::default()), rect, 10.0).unwrap();
    assert_eq!(centred.scale, 0.1);
    assert_eq!(centred.to_screen(250.0, 0.0), [1000.0, 80.0], "the edited glyph's middle at the strip's middle, the baseline 800 units down");
    assert_eq!(hit(&line_model(PreviewOptions::default()), &centred, [1000.0, 50.0]), Some(0));

    let flush = place(&line_model(PreviewOptions { always_center: false, offset: 5.0, ..Default::default() }), rect, 10.0).unwrap();
    assert_eq!(flush.to_screen(0.0, 0.0)[0], 15.0, "flush with the left edge after the margin, then shifted");

    let turned = place(&line_model(PreviewOptions { flip: true, ..Default::default() }), rect, 10.0).unwrap();
    let p = turned.to_screen(100.0, 300.0);
    assert_eq!(turned.to_font(p), [100.0, 300.0]);
    assert_eq!(p, [2000.0 - centred.to_screen(100.0, 300.0)[0], 100.0 - centred.to_screen(100.0, 300.0)[1]]);
    assert!(place(&line_model(PreviewOptions::default()), [0.0, 0.0, 0.0, 100.0], 10.0).is_none());
}

#[test]
fn the_controls_post_their_commands_and_the_strip_scrolls_and_edits_on_double_click() {
    let model = Tracked::new(line_model(PreviewOptions::default()));
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut f = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show_area(ui, Props { model: &model, target: Target::Area, sent: &mut sent }, 200.0);
        seen.borrow_mut().extend(sent);
    });
    f.room(Vec2::new(1000.0, 200.0)).settle();

    let invert = f.by_label("Invert").expect("the Invert button");
    f.click(&invert).settle();
    assert_eq!(posted.borrow().as_slice(), [shell("preview.invert", json!({"target": "area", "on": true}))]);
    posted.borrow_mut().clear();

    let strip = f.by_label("Preview Area").expect("the strip");
    // The strip takes the room the controls leave; the controls stay inside the area.
    let invert = f.by_label("Invert").expect("the Invert button");
    assert!(strip.rect.height() > 100.0 && strip.rect.bottom() <= invert.rect.top(), "{:?} {:?}", strip.rect, invert.rect);
    assert!(invert.rect.bottom() <= 200.0, "{:?} {:?}", strip.rect, invert.rect);
    // Always Center Active Glyph puts the edited glyph (item 0) in the strip's middle.
    let middle = strip.rect.center();
    f.click_at(middle).click_at(middle).settle();
    assert!(posted.borrow().contains(&shell("editView.setCursor", json!({"index": 0}))), "{:?}", posted.borrow());
    posted.borrow_mut().clear();

    f.drag_at(middle, Vec2::new(30.0, 0.0)).settle();
    let dx: f64 = posted
        .borrow()
        .iter()
        .filter_map(|m| match m {
            Msg::Shell(c) if c.id == "preview.setOffset" => c.params.get("dx").and_then(Value::as_f64),
            _ => None,
        })
        .sum();
    assert!((dx - 30.0).abs() < 0.5, "dragging shifts the line by the drag: {dx}");
    posted.borrow_mut().clear();

    let menu = f.by_label("-").expect("the instance pop-up showing the dash");
    f.click(&menu).settle();
    let light = f.by_label("Light").expect("the instance in the pop-up");
    f.click(&light).settle();
    assert_eq!(posted.borrow().as_slice(), [shell("preview.setInstance", json!({"target": "area", "instance": 0}))]);
}
