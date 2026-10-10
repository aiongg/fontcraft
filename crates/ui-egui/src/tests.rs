//! Headless UI tests: frames are run in a bare egui context, with no window.

use fontcraft_engine::command_specs;
use serde_json::{Value, json};

use crate::control;
use crate::shell::UI_COMMANDS;
use crate::shell::menus::{MENU_ORDER, menu_tree};
use crate::{FontcraftApp, theme::Tokens};

/// Runs `frames` frames of the app at 1280 × 800 and returns how many shapes the last one drew.
fn frames(app: &mut FontcraftApp, frames: usize) -> usize {
    let ctx = egui::Context::default();
    let input = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut shapes = 0;
    for _ in 0..frames {
        let mut output = ctx.run_ui(input(), |ui| app.ui(ui));
        // No renderer takes the font atlas here; say so, or epaint flags the dropped texture update.
        output.textures_delta.clear();
        shapes = output.shapes.len();
    }
    shapes
}

fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
    control::handle(app, &json!({"id": 1, "method": method, "params": params}))
}

#[test]
fn font_view_and_edit_view_draw_without_a_window() {
    let mut app = FontcraftApp::default();
    let grid = frames(&mut app, 2);
    assert!(grid > 50, "Font View drew only {grid} shapes");
    app.run("path.addPath", &json!({"glyph": "A", "nodes": [{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 310, "y": 700}]})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    let edit = frames(&mut app, 2);
    assert!(edit > 5, "Edit View drew only {edit} shapes");
    // With no document the app still draws.
    app.run("file.close", &json!({"discard": true})).unwrap();
    assert!(app.state.editing().is_none(), "closing the document leaves Edit View");
    frames(&mut app, 1);
}

#[test]
fn every_menu_item_is_a_command_and_the_bar_is_in_order() {
    let app = FontcraftApp::default();
    let tree = menu_tree(&app.session);
    let titles: Vec<&str> = tree.iter().map(|m| m.title).collect();
    let mut expected: Vec<&str> = MENU_ORDER.iter().copied().filter(|t| titles.contains(t)).collect();
    expected.extend(titles.iter().filter(|t| !MENU_ORDER.contains(t)));
    assert_eq!(titles, expected);
    let ids: Vec<&str> = command_specs().iter().map(|c| c.id).chain(UI_COMMANDS.iter().map(|c| c.0)).collect();
    for item in tree.iter().flat_map(|m| &m.items) {
        assert!(ids.contains(&item.command), "{} is in a menu but is not a command", item.command);
    }
    let undo = tree.iter().flat_map(|m| &m.items).find(|i| i.command == "edit.undo").unwrap();
    assert!(!undo.enabled && undo.disabled_reason.as_deref() == Some("nothing to undo"));
}

#[test]
fn ui_and_engine_command_ids_do_not_clash() {
    let mut seen = std::collections::HashSet::new();
    for id in command_specs().iter().map(|c| c.id).chain(UI_COMMANDS.iter().map(|c| c.0)) {
        assert!(seen.insert(id), "{id} is registered twice");
    }
    // A shortcut may repeat when the copies work in different places (`when`); UI commands work
    // everywhere.
    let engine = command_specs().iter().filter_map(|c| Some((c.shortcut?, c.when, c.id)));
    let ui = UI_COMMANDS.iter().filter(|c| !c.3.is_empty()).map(|c| (c.3, None, c.0));
    let mut shortcuts = std::collections::HashSet::new();
    for (shortcut, when, id) in engine.chain(ui) {
        assert!(shortcuts.insert((shortcut, when)), "{id}: another command has the default shortcut {shortcut} where {when:?}");
    }
}

#[test]
fn the_control_protocol_drives_the_app() {
    let mut app = FontcraftApp::default();
    let r = call(&mut app, "engine.execute", json!({"command": "glyph.newGlyph", "params": {"name": "agent"}}));
    assert_eq!((r["ok"].as_bool(), r["result"]["name"].as_str()), (Some(true), Some("agent")));
    assert_eq!(call(&mut app, "engine.execute", json!({"command": "fontView.setSelection", "params": {"glyphs": ["agent"]}}))["ok"], true);
    assert_eq!(call(&mut app, "ui.inspect", json!({}))["result"]["selected_glyphs"], json!(["agent"]));
    assert_eq!(call(&mut app, "document.inspect", json!({}))["result"]["documents"][0]["undo"], json!(["New Glyph"]));
    let commands = call(&mut app, "engine.commands", json!({}));
    let list = commands["result"]["commands"].as_array().unwrap();
    assert_eq!(list.len(), command_specs().len() + UI_COMMANDS.len());
    assert!(call(&mut app, "ui.menu.list", json!({}))["result"].as_array().unwrap().len() >= 4);
    assert_eq!(call(&mut app, "ui.menu.invoke", json!({"command": "edit.undo"}))["ok"], true);
    assert_eq!(call(&mut app, "ui.menu.invoke", json!({"command": "font.info.get"}))["ok"], false);
    // Removing the selected glyph clears the selection; errors come back as replies.
    assert_eq!(call(&mut app, "ui.inspect", json!({}))["result"]["selected_glyphs"], json!([]));
    for bad in [
        json!({"method": "nope"}),
        json!({"id": 2}),
        json!({"method": 5}),
        json!({"method": "engine.execute"}),
        json!({"method": "engine.execute", "params": {"command": "no.such"}}),
        json!([]),
        json!("x"),
    ] {
        let reply = control::handle(&mut app, &bad);
        assert_eq!(reply["ok"], false, "{bad}");
        assert!(reply["error"].as_str().is_some_and(|e| !e.is_empty()));
    }
}

#[test]
fn ui_commands_reject_junk_without_panicking() {
    let mut app = FontcraftApp::default();
    let junk = [
        Value::Null,
        json!({}),
        json!([]),
        json!("x"),
        json!({"glyphs": 5}),
        json!({"glyphs": [5]}),
        json!({"glyphs": ["nope"]}),
        json!({"glyph": []}),
        json!({"on": "yes"}),
        json!({"tab": -1}),
        json!({"tab": 99}),
        json!({"index": 0}),
        json!({"index": 1e30}),
        json!({"keep": 1000}),
        json!({"size": 0}),
        json!({"size": -5}),
        json!({"size": 1e300}),
        json!({"center": [1]}),
        json!({"center": ["a", 2]}),
        json!({"rect": [0, 0, 0, 10]}),
        json!({"rect": [0, 0, 1e308, 1e308]}),
        json!({"x": 1e308, "dx": -1e308}),
        json!({"text": "\u{1F600}"}),
    ];
    app.run("view.newTab", &json!({"glyphs": ["A", "B"]})).unwrap();
    for (id, ..) in UI_COMMANDS.iter() {
        for params in &junk {
            let _ = app.run(id, params);
            for tab in &app.state.tabs {
                assert!(crate::state::ZOOM_RANGE.contains(&tab.view.scale), "{id} {params}: zoom {}", tab.view.scale);
                assert!(tab.view.center.iter().all(|c| c.is_finite() && c.abs() <= crate::state::MAX_SCROLL), "{id} {params}");
            }
        }
        if app.state.tabs.is_empty() {
            app.run("view.newTab", &json!({"glyphs": ["A"]})).unwrap();
        }
        frames(&mut app, 1);
    }
}

#[test]
fn a_large_font_draws_only_the_rows_in_view() {
    let mut app = FontcraftApp::default();
    let small = frames(&mut app, 2);
    // Twenty thousand more glyphs: the frame must not grow with the font.
    app.session
        .edit("many", |font| {
            for i in 0..20_000 {
                font.add_glyph(&format!("g{i:05}"), vec![], 500.0)?;
            }
            Ok(())
        })
        .unwrap();
    let start = std::time::Instant::now();
    let large = frames(&mut app, 2);
    assert!(large < small * 4, "Font View drew {large} shapes for a large font, {small} for a small one");
    assert!(start.elapsed().as_secs() < 5);
    // Selecting every glyph is quick too.
    let all: Vec<String> = app.session.font().unwrap().glyphs.iter().map(|g| g.name.clone()).collect();
    let start = std::time::Instant::now();
    app.run("fontView.setSelection", &json!({"glyphs": all})).unwrap();
    assert!(start.elapsed().as_secs() < 2);
    assert_eq!(app.state.selected_glyphs.len(), 20_054);
}

#[test]
fn themes_differ_only_where_they_should() {
    let (dark, light) = (Tokens::dark(), Tokens::light());
    assert!(dark.dark && !light.dark);
    assert_ne!(dark.panel, light.panel);
    assert_eq!(dark.accent, light.accent);
}

#[test]
fn ui_render_draws_glyph_pixels_and_screenshots_need_a_window() {
    let mut app = FontcraftApp::default();
    app.run("path.addPath", &json!({"glyph": "A", "nodes": [{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 310, "y": 700}]})).unwrap();
    let render = |app: &mut FontcraftApp, params: Value| call(app, "ui.render", params);
    let first = render(&mut app, json!({"glyphs": ["A"], "size": 100}));
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!((first["result"]["width"].as_u64(), first["result"]["height"].as_u64()), (Some(76), Some(116)));
    assert_eq!(render(&mut app, json!({"glyphs": ["A"], "size": 100}))["result"]["png"], first["result"]["png"], "the same bytes twice");
    // Text maps characters to glyphs; with nothing named, the edited glyph is drawn.
    assert_eq!(render(&mut app, json!({"text": "AB", "scale": 0.1}))["result"]["glyphs"], json!(["A", "B"]));
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    assert_eq!(render(&mut app, json!({}))["result"]["glyphs"], json!(["A"]));
    for bad in [
        json!({"glyphs": ["nope"]}),
        json!({"text": "\u{1F600}"}),
        json!({"size": -1}),
        json!({"scale": 1e9}),
        json!({"glyphs": 5}),
        json!({"path": 5}),
    ] {
        assert_eq!(render(&mut app, bad.clone())["ok"], false, "{bad}");
    }
    let shot = call(&mut app, "ui.screenshot", json!({}));
    assert!(shot["error"].as_str().is_some_and(|e| e.starts_with("ui.screenshot:")), "{shot}");
    match control::handle_request(&mut app, &json!({"id": 7, "method": "ui.screenshot", "params": {"path": "x.png"}})) {
        control::Handled::Screenshot(request) => {
            assert_eq!(request.path.as_deref(), Some("x.png"));
            let reply = control::ScreenshotRequest { path: None, ..request }.finish(2, 1, vec![255; 8], 1.0);
            assert_eq!((reply["id"].as_i64(), reply["result"]["width"].as_u64()), (Some(7), Some(2)));
        }
        other => panic!("ui.screenshot was answered without a window: {other:?}"),
    }
}

/// The shapes of the last of `n` frames, with nested lists flattened.
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

/// A glyph with every display convention: a closed path with corner, smooth and handle nodes
/// whose start and second node sit on the baseline; an open path with a node in the x-height
/// zone; a closed path with two neighbouring nodes at one place.
fn convention_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    let path = |app: &mut FontcraftApp, closed: bool, nodes: Value| {
        app.run("path.addPath", &json!({"glyph": "A", "closed": closed, "nodes": nodes})).unwrap();
    };
    path(
        &mut app,
        true,
        json!([
            {"x": 100, "y": 0, "type": "curve"}, {"x": 500, "y": 0},
            {"x": 500, "y": 200, "type": "offcurve"}, {"x": 400, "y": 300, "type": "offcurve"}, {"x": 300, "y": 300, "type": "curve", "smooth": true},
            {"x": 200, "y": 300, "type": "offcurve"}, {"x": 100, "y": 200, "type": "offcurve"},
        ]),
    );
    path(&mut app, false, json!([{"x": 600, "y": 100}, {"x": 700, "y": 508}, {"x": 800, "y": 100}]));
    path(&mut app, true, json!([{"x": 100, "y": 600}, {"x": 300, "y": 600}, {"x": 300, "y": 600}, {"x": 200, "y": 650}]));
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app
}

#[test]
fn edit_view_draws_every_display_convention_from_the_tokens() {
    let mut app = convention_app();
    let t = Tokens::dark();
    let shapes = drawn(&mut app, 2);
    let count = |f: &dyn Fn(&egui::Shape) -> bool| shapes.iter().filter(|s| f(s)).count();
    let circles = |fill: egui::Color32| count(&|s| matches!(s, egui::Shape::Circle(c) if c.fill == fill));
    let polygons = |fill: egui::Color32| count(&|s| matches!(s, egui::Shape::Path(p) if p.fill == fill));
    let segments = |color: egui::Color32| count(&|s| matches!(s, egui::Shape::LineSegment { stroke, .. } if stroke.color == color));
    let corners = count(&|s| matches!(s, egui::Shape::Rect(r) if r.fill == t.node_corner));
    assert_eq!(corners, 7, "corner squares (start nodes are arrows instead)");
    assert_eq!(circles(t.node_smooth), 1, "smooth circles");
    assert_eq!(circles(t.node_handle), 4, "handles");
    assert_eq!(segments(t.handle_line), 4, "handle lines, one per handle");
    assert_eq!(polygons(t.node_corner), 2, "start arrows of the two closed paths");
    assert_eq!(segments(t.open_end), 2, "strokes across the open path's ends");
    assert_eq!(polygons(t.open_end_start), 1, "the open path's first-node triangle");
    assert_eq!(count(&|s| matches!(s, egui::Shape::Circle(c) if c.stroke.color == t.duplicate_node)), 1, "the coincident pair");
    assert_eq!(polygons(t.metric_highlight), 2, "diamonds behind the two nodes on the baseline");
    assert_eq!(circles(t.metric_highlight), 1, "a circle behind the node in the x-height zone");
    assert!(segments(t.metric_line) >= 7, "five metric lines and two side lines");
    // Without nodes, none of that is drawn; without metrics, no lines.
    app.run("view.showNodes", &json!({"on": false})).unwrap();
    app.run("view.showMetrics", &json!({})).unwrap();
    let bare = drawn(&mut app, 1);
    assert!(!bare.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.fill == t.node_corner)));
    assert!(!bare.iter().any(|s| matches!(s, egui::Shape::LineSegment { stroke, .. } if stroke.color == t.metric_line)));
}

#[test]
fn zoom_and_scroll_commands_move_the_camera_within_limits() {
    let mut app = convention_app();
    let options = |app: &mut FontcraftApp| app.run("view.getDisplayOptions", &json!({})).unwrap();
    let fitted = options(&mut app);
    // The tab opened fitted: descender to ascender (1000 units) in 720 points less the margins.
    let zoom = fitted["zoom"].as_f64().unwrap();
    assert!((zoom - 720.0 * (1.0 - 2.0 * 0.08)).abs() < 0.5, "{fitted}");
    assert_eq!(fitted["center"], json!([300.0, 300.0]));
    assert_eq!((fitted["nodes"].as_bool(), fitted["direction"].as_str()), (Some(true), Some("ltr")));
    app.run("view.setZoom", &json!({"size": 1000, "center": [0, 0]})).unwrap();
    app.run("view.zoomIn", &json!({})).unwrap();
    let z = options(&mut app)["zoom"].as_f64().unwrap();
    assert!((z - 1000.0 * std::f64::consts::SQRT_2).abs() < 1e-6);
    app.run("view.scrollTo", &json!({"dx": 100, "dy": -50})).unwrap();
    let center = options(&mut app)["center"].clone();
    let s = z / 1000.0;
    assert!((center[0].as_f64().unwrap() + 100.0 / s).abs() < 1e-6 && (center[1].as_f64().unwrap() + 50.0 / s).abs() < 1e-6, "{center}");
    app.run("view.zoomToRect", &json!({"rect": [0, 0, 100, 100]})).unwrap();
    assert_eq!(options(&mut app)["center"], json!([50.0, 50.0]));
    for _ in 0..100 {
        app.run("view.zoomOut", &json!({})).unwrap();
    }
    assert_eq!(options(&mut app)["zoom"].as_f64(), Some(crate::state::ZOOM_RANGE.start() * 1000.0));
    app.run("view.setZoom", &json!({"size": 1e9})).unwrap();
    assert_eq!(options(&mut app)["zoom"].as_f64(), Some(crate::state::ZOOM_RANGE.end() * 1000.0));
    app.run("view.zoomToActualSize", &json!({})).unwrap();
    assert_eq!(options(&mut app)["zoom"].as_f64(), Some(1000.0));
    // Toggles flip, or take `on`.
    assert_eq!(app.run("view.showGuides", &json!({})).unwrap(), json!({"on": false}));
    assert_eq!(app.run("view.showGuides", &json!({"on": true})).unwrap(), json!({"on": true}));
    // In Font View there is no camera to move.
    app.run("fontView.show", &json!({})).unwrap();
    assert!(app.run("view.zoomIn", &json!({})).unwrap_err().contains("no Edit View tab"));
}

#[test]
fn tabs_open_switch_close_and_reopen() {
    let mut app = FontcraftApp::default();
    let tabs = |app: &FontcraftApp| app.state.tabs.iter().map(|t| t.glyphs.join(" ")).collect::<Vec<_>>();
    app.run("fontView.setSelection", &json!({"glyphs": ["A", "B"]})).unwrap();
    let first = app.run("view.newTab", &json!({})).unwrap();
    assert_eq!((first["position"].as_u64(), first["editing"].as_str()), (Some(2), Some("A")));
    app.run("view.newTab", &json!({"text": "CA"})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A", "B"]})).unwrap();
    assert_eq!((tabs(&app), app.state.active_tab), (vec!["A B".to_string(), "C A".into()], Some(0)), "an open tab is reused");
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["D"]})).unwrap();
    assert_eq!(app.state.tabs.len(), 3);
    app.run("view.showNextTab", &json!({})).unwrap();
    assert_eq!(app.state.active_tab, None, "after the last tab comes Font View");
    app.run("view.showPreviousTab", &json!({})).unwrap();
    assert_eq!(app.state.active_tab, Some(2));
    app.run("view.showTab", &json!({"index": 3})).unwrap();
    assert_eq!(app.state.editing(), Some("C"));
    app.run("view.closeTab", &json!({})).unwrap();
    assert_eq!((tabs(&app).len(), app.state.active_tab), (2, Some(0)), "the tab to the left shows");
    app.run("view.reopenLastClosedTab", &json!({})).unwrap();
    assert_eq!((tabs(&app)[1].as_str(), app.state.active_tab), ("C A", Some(1)), "back where it was");
    app.run("view.closeOtherTabs", &json!({"keep": 4})).unwrap();
    assert_eq!((tabs(&app), app.state.active_tab), (vec!["D".to_string()], Some(0)));
    assert!(app.run("view.closeTab", &json!({"index": 1})).is_err(), "Font View stays");
    assert!(app.run("view.showTab", &json!({"index": 3})).is_err());
    // A tab keeps its own camera.
    app.run("view.newTab", &json!({"glyphs": ["E"]})).unwrap();
    app.run("view.zoomToActualSize", &json!({})).unwrap();
    app.run("view.showTab", &json!({"index": 2})).unwrap();
    assert_ne!(app.state.tab().map(|t| t.view.scale), Some(1.0));
    // Removing a glyph takes it out of its tabs; a tab left empty closes.
    app.run("glyph.removeGlyph", &json!({"glyphs": ["D"]})).unwrap();
    assert_eq!(tabs(&app), vec!["E".to_string()]);
    assert_eq!(app.state.active_tab, None);
    frames(&mut app, 1);
}

#[test]
fn menu_shortcuts_parse_and_run_their_commands() {
    use crate::shell::keymap::parse;
    let s = parse("Cmd+Alt+Shift+W").unwrap();
    assert!(s.modifiers.command && s.modifiers.alt && s.modifiers.shift && s.logical_key == egui::Key::W);
    assert_eq!(parse("Cmd+Plus").map(|s| s.logical_key), Some(egui::Key::Plus));
    assert!(parse("").is_none() && parse("Cmd+Nope").is_none());
    let mut app = convention_app();
    let before = app.state.tab().map(|t| t.view.scale).unwrap();
    let ctx = egui::Context::default();
    let key = |key: egui::Key, modifiers: egui::Modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        events: vec![key(egui::Key::Plus, egui::Modifiers::COMMAND)],
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| app.ui(ui));
    output.textures_delta.clear();
    let after = app.state.tab().map(|t| t.view.scale).unwrap();
    assert!((after / before - std::f64::consts::SQRT_2).abs() < 1e-9, "Cmd+Plus zooms in: {before} → {after}");
}
