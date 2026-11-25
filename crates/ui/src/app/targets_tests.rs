//! Which glyph and which layer a fired command changes, in each place the person can fire it from.

use std::collections::BTreeMap;
use std::sync::Arc;

use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::model::Layer;
use fontcraft_geom::Point;
use fontcraft_tools::{Item, Selection};
use serde_json::{Value, json};

use super::BATCH;
use crate::app::context_menu::{self, Space};
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::canvas::text::TEXT_TOOL;

fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
    app.status.clear();
    app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
    app.drain();
    assert!(app.status.is_empty(), "{id}: {}", app.status);
}

/// Glyphs A, B, C with a square off the middle and the metrics key `=H` out of date, in two masters; B has a third layer
/// `bk`. A is shown in the edit view, in the second master, `m02`.
fn app() -> FontCraftApp {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.session
        .edit("setup", |font| {
            let mut m = font.default_master().unwrap().clone();
            m.id = "m02".into();
            m.name = "Bold".into();
            Arc::make_mut(&mut font.masters).push(m);
            let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
            for name in names {
                font.glyph_mut(&name)?.layers.push(Layer::new("m02", 600.0));
            }
            Ok(())
        })
        .unwrap();
    let first = app.session.font().unwrap().masters[0].id.clone();
    let mut run = |id: &str, params: Value| app.session.execute(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
    for layer in [first.as_str(), "m02"] {
        for (glyph, x) in [("A", 100), ("B", 110), ("C", 120), ("H", 40)] {
            run("path.addRectangle", json!({"glyph": glyph, "layer": layer, "x": x, "y": 100, "width": 200, "height": 200}));
        }
        run("spacing.setLeftSidebearing", json!({"glyphs": ["A", "B", "C"], "layer": layer, "value": "=H"}));
    }
    for layer in [first.as_str(), "m02"] {
        run("path.addRectangle", json!({"glyph": "H", "layer": layer, "x": 10, "y": 100, "width": 20, "height": 20}));
    }
    app.session
        .edit("setup", |font| {
            let mut backup = font.glyph("B").unwrap().layer("m02").unwrap().clone();
            backup.id = "bk".into();
            font.glyph_mut("B")?.layers.push(backup);
            Ok(())
        })
        .unwrap();
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    shell(&mut app, "view.selectMaster", json!({"master": "m02"}));
    app
}

/// Every layer of the font as text, by glyph and layer id.
fn layers(app: &FontCraftApp) -> BTreeMap<(String, String), String> {
    let font = app.session.font().unwrap();
    font.glyphs.iter().flat_map(|g| g.layers.iter().map(|l| ((g.name.clone(), l.id.clone()), format!("{l:?}"))).collect::<Vec<_>>()).collect()
}

/// The layers that differ from `before`.
fn changed(app: &FontCraftApp, before: &BTreeMap<(String, String), String>) -> Vec<(String, String)> {
    let now = layers(app);
    before.iter().filter(|(key, text)| now.get(*key) != Some(*text)).map(|(key, _)| key.clone()).collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(g, l)| (g.to_string(), l.to_string())).collect()
}

/// Text mode with the text A, B, C: A and B selected, B in its own layer `bk`.
fn select_text(app: &mut FontCraftApp) {
    app.sent.push(Msg::View(ViewMsg::SelectTool(TEXT_TOOL)));
    app.drain();
    shell(app, "editView.setText", json!({"glyphs": ["A", "B", "C"]}));
    assert!(app.view.text.text.set_layer(1, Some("bk")));
    shell(app, "editView.setCursor", json!({"index": 0, "selectionLength": 2}));
}

/// Commands that change the layer they are given, and whether they also do on a layer that is no master's.
const CHANGING: &[(&str, bool)] = &[
    ("glyph.updateMetrics", false),
    ("spacing.centerInWidth", true),
    ("glyph.setAnchors", true),
    ("glyph.resetAnchors", true),
    ("path.reverseContours", true),
    ("path.swapWithBackground", true),
];

/// The edit view: only the shown layer of the edited glyph changes.
#[test]
fn a_fired_command_changes_the_layer_the_edit_view_shows() {
    for &(id, _) in CHANGING {
        let mut app = app();
        let before = layers(&app);
        let command = app.with_targets(CommandRef::new(id));
        app.run_now(Msg::Run(command), false).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(changed(&app, &before), pairs(&[("A", "m02")]), "{id}");
    }
}

/// Text mode: each selected glyph changes in the layer its item shows, as one undo step.
#[test]
fn a_fired_command_changes_each_selected_text_glyph_in_its_own_layer() {
    for &(id, any_layer) in CHANGING {
        let mut app = app();
        select_text(&mut app);
        let before = layers(&app);
        let command = app.with_targets(CommandRef::new(id));
        assert_eq!(command.id, BATCH, "{id} runs once per layer");
        if !any_layer {
            continue;
        }
        app.run_now(Msg::Run(command), false).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(changed(&app, &before), pairs(&[("A", "m02"), ("B", "bk")]), "{id}");
        app.session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(changed(&app, &before), pairs(&[]), "{id} is one undo step");
    }
    // One layer for every glyph: one run. A glyph typed twice is named once, a command for all masters names no layer.
    let mut app = app();
    app.sent.push(Msg::View(ViewMsg::SelectTool(TEXT_TOOL)));
    app.drain();
    shell(&mut app, "editView.setText", json!({"glyphs": ["A", "B", "A", "C"]}));
    shell(&mut app, "editView.setCursor", json!({"index": 0, "selectionLength": 3}));
    let one = app.with_targets(CommandRef::new("path.tidyUpPaths"));
    assert_eq!(one.params, json!({"glyphs": ["A", "B"], "layer": "m02"}));
    let all = app.with_targets(CommandRef::new("path.tidyUpPathsForAllMasters"));
    assert_eq!(all.params, json!({"glyphs": ["A", "B"]}));
    // Only a cursor: the edited glyph.
    shell(&mut app, "editView.setCursor", json!({"index": 3}));
    assert_eq!(app.with_targets(CommandRef::new("glyph.updateMetrics")).params, json!({"glyphs": ["C"], "layer": "m02"}));
    // A locked glyph is left out and named.
    app.session.execute("glyph.toggleLocked", &json!({"glyphs": ["B"]})).unwrap();
    shell(&mut app, "editView.setCursor", json!({"index": 0, "selectionLength": 3}));
    app.status.clear();
    app.fire(CommandRef::new("path.tidyUpPaths"));
    app.drain();
    assert!(app.status.ends_with(": B"), "{}", app.status);
}

/// A dialog fired on a text selection shown in two layers previews and keeps the change in both.
#[test]
fn a_dialog_on_a_text_selection_changes_each_glyph_in_its_own_layer() {
    let mut app = app();
    select_text(&mut app);
    let before = layers(&app);
    crate::dialogs::form::open_command(&mut app, &json!({"dialog": "dialog.filterRoundCorners"})).unwrap();
    crate::dialogs::form::confirm(&mut app).unwrap();
    assert_eq!(changed(&app, &before), pairs(&[("A", "m02"), ("B", "bk")]));
}

/// Font View: the selected glyphs change in the current master, never the glyph of the tab behind it; with nothing
/// selected nothing changes and the status line says what is missing.
#[test]
fn a_fired_command_changes_the_font_view_selection_only() {
    for &(id, _) in CHANGING {
        let mut app = app();
        shell(&mut app, "fontView.show", json!({}));
        let before = layers(&app);
        app.status.clear();
        app.fire(CommandRef::new(id));
        app.drain();
        let label = fontcraft_engine::find_command(id).unwrap().label;
        if app.dialogs.open.is_none() {
            assert_eq!(app.status, format!("{label}: select a glyph first"));
        }
        assert_eq!(changed(&app, &before), pairs(&[]), "{id} with nothing selected");

        shell(&mut app, "fontView.setSelection", json!({"glyphs": ["B", "C"]}));
        let command = app.with_targets(CommandRef::new(id));
        app.run_now(Msg::Run(command), false).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(changed(&app, &before), pairs(&[("B", "m02"), ("C", "m02")]), "{id}");
    }
    // Commands on nodes or on one layer have no target there, whatever the glyph behind Font View has selected.
    let mut app = app();
    app.tools.selection = Selection::from_items([Item::Point(0, 0), Item::Point(0, 1)]);
    shell(&mut app, "fontView.show", json!({}));
    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["B", "C"]}));
    let before = layers(&app);
    app.status.clear();
    app.fire(CommandRef::new("path.collapsePoints"));
    app.drain();
    assert_eq!(app.status, "Collapse Points: edit a glyph and select nodes first");
    assert_eq!(changed(&app, &before), pairs(&[]));
}

/// No key the control channel plays while Font View shows reaches the glyph behind it.
#[test]
fn a_played_key_with_nothing_selected_in_font_view_refuses() {
    let mut app = app();
    shell(&mut app, "fontView.show", json!({}));
    let before = layers(&app);
    let _ = crate::control::dispatch(&mut app, "ui.key", &json!({"key": "t", "modifiers": ["cmd", "shift"]}));
    app.drain();
    assert_eq!(changed(&app, &before), pairs(&[]));
}

/// Every menu command whose parameters name `layer?` gets one in the edit view, in a text selection and in Font View;
/// the exceptions act on every master by default.
#[test]
fn every_menu_command_on_a_layer_is_told_which() {
    let mut app = app();
    let specs: Vec<_> = fontcraft_engine::cmd::command_specs()
        .iter()
        .filter(|s| !s.menu.is_empty() && s.targets.contains(&fontcraft_engine::params::TargetKind::Glyphs) && s.params.contains("layer?"))
        .filter(|s| s.params.split('→').next().is_some_and(|given| given.contains("layer?")))
        .collect();
    assert!(specs.len() > 20, "{}", specs.len());
    let names_layer = |command: &CommandRef| ["layer", "layers"].iter().any(|k| command.params.get(*k).is_some()) || command.id == BATCH;
    let without = |app: &mut FontCraftApp| -> Vec<&'static str> {
        specs.iter().filter(|s| !names_layer(&app.with_targets(CommandRef::new(s.id)))).map(|s| s.id).collect()
    };
    const EVERY_MASTER: [&str; 1] = ["font.detectComposites"];
    assert_eq!(without(&mut app), EVERY_MASTER, "the edit view");
    select_text(&mut app);
    assert_eq!(without(&mut app), EVERY_MASTER, "a text selection");
    shell(&mut app, "fontView.show", json!({}));
    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["B", "C"]}));
    assert_eq!(without(&mut app), EVERY_MASTER, "Font View");
    let metrics = app.with_targets(CommandRef::new("edit.copyMetrics"));
    assert_eq!(metrics.params["layers"], json!(["m02"]), "a command with `layers?` alone");
    let backup = app.with_targets(CommandRef::new("layers.addBackupLayer"));
    assert_eq!(backup.params["master"], json!("m02"), "a command with `master?`");
}

/// A component picked while Font View shows goes into every selected glyph, not into the glyph behind it.
#[test]
fn a_picked_component_goes_into_the_font_view_selection() {
    let mut app = app();
    shell(&mut app, "fontView.show", json!({}));
    app.status.clear();
    app.fire(CommandRef::new("glyph.addComponent"));
    assert!(app.view.components.picker.is_none());
    assert_eq!(app.status, crate::app::select_glyph_first("glyph.addComponent"));

    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["B", "C"]}));
    app.fire(CommandRef::new("glyph.addComponent"));
    let Msg::Run(pick) = app.view.components.picker.clone().expect("the picker").pick().unwrap() else { panic!("an engine command") };
    let steps = pick.params["steps"].as_array().unwrap();
    let into: Vec<(&Value, &Value)> = steps.iter().map(|s| (&s["params"]["glyph"], &s["params"]["layer"])).collect();
    assert_eq!(into, [(&json!("B"), &json!("m02")), (&json!("C"), &json!("m02"))]);
}

/// The canvas menu in text mode lists glyph items; they and the swatches act on the selected glyphs.
#[test]
fn the_canvas_menu_in_text_mode_acts_on_the_selected_glyphs() {
    let mut app = app();
    select_text(&mut app);
    context_menu::open_canvas(&mut app, Point::new(150.0, 150.0), Space::Font).unwrap();
    let menu = app.context_menus.canvas.clone().expect("the menu");
    assert_eq!(menu.glyphs, ["A", "B"]);
    let items: Vec<&str> = menu.sections.iter().flat_map(|s| s.items.iter().map(|i| i.command)).collect();
    assert!(items.contains(&"glyph.toggleLocked") && !items.contains(&"guide.add"), "{items:?}");
    let before = layers(&app);
    for msg in context_menu::color_steps(&app, &menu, Some(3), true) {
        app.run_now(msg, false).unwrap();
    }
    assert_eq!(changed(&app, &before), pairs(&[("A", "m02"), ("B", "bk")]), "the layer swatches");
    for msg in context_menu::color_steps(&app, &menu, Some(4), false) {
        app.run_now(msg, false).unwrap();
    }
    let font = app.session.font().unwrap();
    let labelled: Vec<&str> = font.glyphs.iter().filter(|g| g.info.color.is_some()).map(|g| g.name.as_str()).collect();
    assert_eq!(labelled, ["A", "B"]);
}

/// With Font View in front, Transformations and Fit Curve act on its selection; with nothing selected they have no
/// targets, and no panel shows the glyph behind Font View.
#[test]
fn the_palette_acts_on_the_font_view_selection() {
    let mut app = app();
    app.tools.selection = Selection::from_items([Item::Point(0, 0), Item::Point(0, 1)]);
    app.drain();
    shell(&mut app, "fontView.show", json!({}));
    assert!(app.models.palette.transform.targets.is_none() && app.models.palette.fit_curve.targets.is_none());
    assert!(app.models.palette.info_box.glyph.is_none());
    assert_eq!((app.models.glyph_metrics.glyph.clone(), app.models.layers.glyphs.len()), (None, 0));

    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["B", "C"]}));
    assert_eq!((app.models.glyph_metrics.glyph.clone(), app.models.layers.glyphs.clone()), (None, vec!["B".into(), "C".into()]), "several glyphs");
    let before = layers(&app);
    let slant = app.models.palette.transform.targets.clone().expect("targets").run("transform.slantRight", json!({"angle": 10}));
    app.run_now(slant, false).unwrap();
    assert_eq!(changed(&app, &before), pairs(&[("B", "m02"), ("C", "m02")]));
    app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(changed(&app, &before), pairs(&[]), "one undo step");

    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["A", "C"]}));
    let fit = (*app.models.palette.fit_curve).clone();
    assert_eq!(fit.skipped, 1, "no node of C was selected");
    let targets = fit.targets.expect("A's nodes").with(json!({}));
    assert_eq!((&targets["glyph"], &targets["layer"], &targets["points"]), (&json!("A"), &json!("m02"), &json!([[0, 0], [0, 1]])));

    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["C"]}));
    let shown = (app.models.glyph_metrics.glyph.clone(), app.models.glyph_metrics.layer.clone());
    assert_eq!(shown, (Some("C".into()), Some("m02".into())), "one selected glyph shows in the panels");
}
