//! Caps, segment components and brushes on the canvas (M13.42): highlight, click and Shift-click, Delete, the Info box
//! parts and their pickers, the context menu's items, and copy and paste onto segments. Each gesture is checked against
//! the command it runs.

use std::sync::Arc;

use craft::behavior::CanvasView;
use craft::behavior::kurbo::Point as KPoint;
use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use egui::{Pos2, Rect, vec2};
use fontcraft_engine::Session;
use fontcraft_engine::model::{Layer, Shape};
use fontcraft_geom::ParamCurve as _;
use fontcraft_tools::{Item, Selection};
use serde_json::{Value, json};

use crate::app::context_menu::item_state;
use crate::app::models::components::{attachment_bands, cap_box, segment_box};
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};

/// The hint indices of the three attachments [`attach_app`] adds.
const CAP: usize = 0;
const SEGMENT: usize = 1;
const BRUSH: usize = 2;

fn run(app: &mut FontCraftApp, id: &str, params: Value) {
    app.sent.push(Msg::run(id, params));
    app.drain();
    assert!(app.status.is_empty(), "{id}: {}", app.status);
}

fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
    app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
    app.drain();
    assert!(app.status.is_empty(), "{id}: {}", app.status);
}

fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
    crate::control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
}

fn layer(app: &FontCraftApp) -> Layer {
    let font = app.session.font().unwrap();
    font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap().clone()
}

fn undo_steps(app: &FontCraftApp) -> usize {
    app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
}

fn selection(app: &FontCraftApp) -> Value {
    serde_json::to_value(&app.tools.selection).unwrap()
}

/// Adds glyph `name` with one path of `nodes` and the anchors `anchors`.
fn add_glyph(app: &mut FontCraftApp, name: &str, closed: bool, nodes: &[(f64, f64)], anchors: &[(&str, f64, f64)]) {
    run(app, "glyph.newGlyph", json!({ "name": name }));
    let nodes: Vec<Value> = nodes.iter().map(|&(x, y)| json!({"x": x, "y": y})).collect();
    run(app, "path.addPath", json!({"glyph": name, "closed": closed, "nodes": nodes}));
    for &(anchor, x, y) in anchors {
        run(app, "anchor.add", json!({"glyph": name, "name": anchor, "x": x, "y": y}));
    }
}

/// Glyph A with a 200-unit square (nodes (100, 100), (300, 100), (300, 300), (100, 300)): `_cap.flag` with Fit on its
/// top segment (hint 0), `_segment.wave` on its bottom segment (hint 1) and `_brush.bar` on its right segment (hint 2),
/// shown at one point per unit. `_segment.peak` and `_brush.wide` are spare glyphs of their kinds.
fn attach_app() -> FontCraftApp {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    add_glyph(&mut app, "_cap.flag", false, &[(0.0, 20.0), (-40.0, -30.0), (100.0, -30.0), (100.0, 20.0)], &[("origin", 0.0, 0.0)]);
    let ends = [("start", 0.0, 0.0), ("end", 200.0, 0.0)];
    add_glyph(&mut app, "_segment.wave", false, &[(0.0, 0.0), (100.0, 60.0), (200.0, 0.0)], &ends);
    add_glyph(&mut app, "_segment.peak", false, &[(0.0, 0.0), (100.0, 90.0), (200.0, 0.0)], &ends);
    let bar = [(0.0, -20.0), (200.0, -20.0), (200.0, 20.0), (0.0, 20.0)];
    add_glyph(&mut app, "_brush.bar", true, &bar, &ends);
    add_glyph(&mut app, "_brush.wide", true, &[(0.0, -30.0), (200.0, -30.0), (200.0, 30.0), (0.0, 30.0)], &ends);
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    run(&mut app, "path.addRectangle", json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200}));
    run(&mut app, "capComponent.add", json!({"glyph": "A", "points": [[0, 2], [0, 3]], "cap": "_cap.flag"}));
    run(&mut app, "capComponent.setFit", json!({"glyph": "A", "caps": [CAP], "fit": true}));
    run(&mut app, "segmentComponent.add", json!({"glyph": "A", "segments": [[0, 1]], "segmentGlyph": "_segment.wave"}));
    run(&mut app, "brush.add", json!({"glyph": "A", "segments": [[0, 2]], "brush": "_brush.bar"}));
    app.view.canvas = CanvasView::new(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0)), KPoint::new(400.0, 400.0), 1.0);
    app
}

/// A point on attachment `hint`'s fitted outline at least 20 units from every node of the layer, in font units: a click
/// there hits the attachment and no node.
fn attach_point(app: &FontCraftApp, hint: usize) -> (f64, f64) {
    let font = app.session.font().unwrap();
    let scene = super::scene(font, app.session.glyph_data().ok().as_deref(), &app.view, None).unwrap();
    let mark = scene.attachments.iter().find(|m| m.hint == hint).unwrap_or_else(|| panic!("attachment {hint} is fitted"));
    let nodes: Vec<(f64, f64)> = scene
        .layer
        .shapes
        .iter()
        .filter_map(|s| if let Shape::Path(p) = s { Some(p) } else { None })
        .flat_map(|p| p.nodes.iter().map(|n| (n.x, n.y)))
        .collect();
    let far = |x: f64, y: f64| nodes.iter().all(|&(nx, ny)| (nx - x).hypot(ny - y) > 20.0);
    mark.outline
        .segments()
        .flat_map(|seg| [0.25, 0.5, 0.75].map(|t| seg.eval(t)))
        .find(|p| far(p.x, p.y))
        .map(|p| (p.x, p.y))
        .unwrap_or_else(|| panic!("attachment {hint} has no point away from the nodes"))
}

fn click(app: &mut FontCraftApp, (x, y): (f64, f64), mods: Value) {
    let events = json!([{"kind": "down", "x": x, "y": y, "mods": mods}, {"kind": "up", "x": x, "y": y, "mods": mods}]);
    call(app, "ui.pointer", json!({ "events": events }));
}

/// Each kind is highlighted along what it draws; a click on that outline selects it and Shift-click adds the others;
/// Delete removes all three in one undo step, as `edit.delete` with `hints` does (`cap.highlightAndSelection`,
/// `segment.highlightAndSelection`, `brush.highlightAndSelection`).
#[test]
fn attachments_are_highlighted_clicked_and_deleted_like_the_command() {
    let mut app = attach_app();
    let font = app.session.font().unwrap();
    let bands: Vec<usize> = attachment_bands(font, &layer(&app)).iter().map(|b| b.hint).collect();
    assert_eq!(bands, vec![CAP, SEGMENT, BRUSH], "every kind has its highlight band");

    let segment = attach_point(&app, SEGMENT);
    click(&mut app, segment, json!([]));
    assert_eq!(selection(&app), json!({ "hints": [SEGMENT] }));
    let cap = attach_point(&app, CAP);
    click(&mut app, cap, json!(["shift"]));
    let brush = attach_point(&app, BRUSH);
    click(&mut app, brush, json!(["shift"]));
    assert_eq!(selection(&app), json!({ "hints": [CAP, SEGMENT, BRUSH] }));
    super::apply::sync(&mut app);
    assert_eq!(selection(&app), json!({ "hints": [CAP, SEGMENT, BRUSH] }), "the selection survives a frame of the tools");

    let steps = undo_steps(&app);
    let r = call(&mut app, "ui.key", json!({"key": "Delete"}));
    assert_eq!(r["handled"], true, "{r}");
    assert!(app.status.is_empty(), "{}", app.status);
    assert_eq!(undo_steps(&app), steps + 1, "one undo step");
    assert!(layer(&app).info.hints.is_empty());
    let mut headless = attach_app();
    run(&mut headless, "edit.delete", json!({"glyph": "A", "hints": [CAP, SEGMENT, BRUSH]}));
    assert_eq!(layer(&app), layer(&headless), "the key gives the font the command gives");
}

/// The segment and brush parts of the Info box show for the selected one; a pick in the picker the name opens runs the
/// kind's `setGlyph`, Cancel changes nothing, and the arrow's command opens the glyph (`infoBox.segmentComponent`,
/// `infoBox.brush`, `segmentComponent.editGlyph`, `brush.editGlyph`).
#[test]
fn info_box_edits_run_the_commands_and_cancel_changes_nothing() {
    let mut app = attach_app();
    shell(&mut app, "selection.set", json!({ "hints": [SEGMENT] }));
    let font = app.session.font().unwrap();
    let l = layer(&app);
    let b = segment_box(font, "A", &l, &app.tools.selection, false).expect("the segment part shows");
    assert_eq!((b.hint, b.name.as_str(), b.exists, b.brush), (SEGMENT, "_segment.wave", true, false));
    assert!(segment_box(font, "A", &l, &app.tools.selection, true).is_none(), "no brush is selected");
    assert!(cap_box(font, "A", &l, &app.tools.selection).is_none(), "no cap is selected");

    shell(&mut app, crate::dialogs::glyph_picker::COMMAND, json!({"purpose": "segmentGlyph"}));
    let st = app.view.components.picker.clone().expect("the picker opens");
    assert_eq!(st.targets.get("segmentComponents"), Some(&json!([SEGMENT])));
    let mut names: Vec<&str> = st.results.iter().map(|r| r.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["_segment.peak", "_segment.wave"], "only segment glyphs are listed");
    shell(&mut app, crate::dialogs::glyph_picker::COMMAND, json!({"select": "_segment.peak", "pick": true}));
    assert!(app.view.components.picker.is_none());
    let mut headless = attach_app();
    run(&mut headless, "segmentComponent.setGlyph", json!({"glyph": "A", "segmentComponents": [SEGMENT], "segmentGlyph": "_segment.peak"}));
    assert_eq!(layer(&app), layer(&headless), "the pick gives the font the command gives");

    let before = layer(&app);
    shell(&mut app, "selection.set", json!({ "hints": [BRUSH] }));
    shell(&mut app, crate::dialogs::glyph_picker::COMMAND, json!({"purpose": "brushGlyph", "select": "_brush.wide"}));
    assert_eq!(app.view.components.picker.as_ref().and_then(|st| st.targets.get("brushes")), Some(&json!([BRUSH])));
    shell(&mut app, crate::dialogs::glyph_picker::COMMAND, json!({"open": false}));
    assert!(app.view.components.picker.is_none());
    assert_eq!(layer(&app), before, "Cancel changes nothing");

    shell(&mut app, "segmentComponent.editGlyph", json!({ "segmentComponent": SEGMENT }));
    assert_eq!(app.view.glyph.as_deref(), Some("_segment.peak"));
    shell(&mut app, "brush.editGlyph", json!({"glyph": "A", "brush": BRUSH}));
    assert_eq!(app.view.glyph.as_deref(), Some("_brush.bar"));
}

/// The Add pickers of the three kinds each list only the glyphs of their kind, and a pick runs the kind's `add`.
#[test]
fn each_add_picker_lists_only_its_kind() {
    let mut app = attach_app();
    for (purpose, targets, listed) in [
        ("addCap", json!({"points": [[0, 0], [0, 1]]}), vec!["_cap.flag"]),
        ("addSegment", json!({"segments": [[0, 3]]}), vec!["_segment.peak", "_segment.wave"]),
        ("addBrush", json!({"segments": [[0, 3]]}), vec!["_brush.bar", "_brush.wide"]),
    ] {
        let mut params = targets.as_object().cloned().unwrap();
        params.insert("purpose".into(), json!(purpose));
        params.insert("glyph".into(), json!("A"));
        shell(&mut app, crate::dialogs::glyph_picker::COMMAND, Value::Object(params));
        let st = app.view.components.picker.clone().expect("the picker opens");
        let mut names: Vec<&str> = st.results.iter().map(|r| r.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, listed, "{purpose}");
        shell(&mut app, crate::dialogs::glyph_picker::COMMAND, json!({"open": false}));
    }
    shell(
        &mut app,
        crate::dialogs::glyph_picker::COMMAND,
        json!({"purpose": "addSegment", "glyph": "A", "segments": [[0, 3]], "select": "_segment.wave", "pick": true}),
    );
    let headless = {
        let mut h = attach_app();
        run(&mut h, "segmentComponent.add", json!({"glyph": "A", "segments": [[0, 3]], "segmentGlyph": "_segment.wave"}));
        layer(&h)
    };
    assert_eq!(layer(&app), headless, "the pick gives the font the command gives");
}

/// The context menu offers Add Cap Component, Add Segment Component and Add Brush only for a selection they can act
/// on, and each Decompose item only for an attachment of its kind.
#[test]
fn context_menu_items_show_only_for_a_valid_selection() {
    let app = attach_app();
    let font = app.session.font().unwrap();
    let scene = super::scene(font, app.session.glyph_data().ok().as_deref(), &app.view, None).unwrap();
    let points = |list: &[(usize, usize)]| Selection::from_items(list.iter().map(|&(s, n)| Item::Point(s, n)));
    let shows = |sel: &Selection, id: &str| item_state::visible(&scene, sel, id);
    assert!(shows(&points(&[(0, 0), (0, 1)]), "capComponent.add"));
    assert!(!shows(&points(&[(0, 0), (0, 2)]), "capComponent.add"), "two nodes of no segment");
    assert!(!shows(&points(&[(0, 0)]), "capComponent.add"));
    assert!(shows(&points(&[(0, 0), (0, 1)]), "segmentComponent.add"));
    assert!(!shows(&points(&[(0, 0)]), "segmentComponent.add"), "one node selects no segment");
    assert!(shows(&points(&[(0, 1)]), "brush.add"), "a node that ends a segment takes a brush");
    let segment = Selection::from_items([Item::Hint(SEGMENT)]);
    assert!(shows(&segment, "segmentComponent.decompose"));
    assert!(!shows(&segment, "capComponent.decompose") && !shows(&segment, "brush.decompose"));
    assert!(shows(&Selection::from_items([Item::Hint(BRUSH)]), "brush.decompose"));
}

/// Copy with a segment component selected copies it; Paste onto one node is refused and changes nothing, Paste onto
/// the two nodes of another segment adds it there, as `edit.copy` and `edit.paste` do (`segment.copyPasteOntoSegments`).
#[test]
fn a_copied_attachment_pastes_only_onto_segments() {
    let mut app = attach_app();
    shell(&mut app, "selection.set", json!({ "hints": [SEGMENT] }));
    let r = call(&mut app, "ui.key", json!({"key": "c", "cmd": true}));
    assert_eq!(r["command"], "edit.copy", "{r}");
    let text = app.clipboard.out.clone().expect("the copy reached the clipboard");
    assert!(text.contains("fontcraft-attachments"), "{text}");

    let before = layer(&app);
    shell(&mut app, "selection.set", json!({"points": [[0, 3]]}));
    crate::app::clipboard::paste(&mut app, &text, crate::app::clipboard::PasteKind::Paste);
    app.drain();
    assert!(!app.status.is_empty(), "pasting onto one node is refused");
    assert_eq!(layer(&app), before, "the refused paste changes nothing");

    app.status.clear();
    shell(&mut app, "selection.set", json!({"points": [[0, 3], [0, 0]]}));
    crate::app::clipboard::paste(&mut app, &text, crate::app::clipboard::PasteKind::Paste);
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
    let mut headless = attach_app();
    let copied =
        headless.session.execute("edit.copy", &json!({"glyph": "A", "segmentComponents": [SEGMENT]})).unwrap()["json"].as_str().unwrap().to_string();
    run(&mut headless, "edit.paste", json!({"glyph": "A", "json": copied, "points": [[0, 3], [0, 0]]}));
    assert_eq!(layer(&app).info.hints, layer(&headless).info.hints, "the paste gives the font the commands give");
    assert_eq!(layer(&app).info.hints.len(), 4);
}
