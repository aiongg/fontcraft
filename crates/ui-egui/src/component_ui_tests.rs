//! The component Info boxes, the component and corner glyph pickers, Component from Selection's
//! dialog and the component context-menu items (M6.11): each filled as a person fills it gives
//! the font its command gives, and Cancel or Escape changes nothing.

use fontcraft_geom::Point;
use fontcraft_model::{Anchor, Component, Font, Glyph, Layer, Shape};
use serde_json::{Value, json};

use crate::panels::{info_box_component, info_box_corner};
use crate::shell::context_menu;
use crate::tools_glue::Space;
use crate::{FontcraftApp, dialogs};

const PICK: &str = "editView.chooseGlyph";

fn master(app: &FontcraftApp) -> String {
    app.session.font().unwrap().masters[0].id.clone()
}

fn layer(app: &FontcraftApp, glyph: &str) -> Value {
    let font = app.session.font().unwrap();
    serde_json::to_value(font.glyph(glyph).unwrap().layer(&font.masters[0].id).unwrap()).unwrap()
}

/// `base`: a square (0,0)–(200,200); `empty`: nothing; `_corner.serif`, `_corner.round`: open
/// paths. `A`: a square (100,100)–(300,300) (shape 0) and components of `base` at (400, 0)
/// (shape 1) and (700, 0) (shape 2). `A` is edited at actual size with Show Info on.
fn app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    for name in ["base", "empty", "_corner.serif", "_corner.round"] {
        app.run("glyph.newGlyph", &json!({"name": name})).unwrap();
    }
    app.run("path.addRectangle", &json!({"glyph": "base", "x": 0, "y": 0, "width": 200, "height": 200})).unwrap();
    for corner in ["_corner.serif", "_corner.round"] {
        app.run("path.addPath", &json!({"glyph": corner, "closed": false, "nodes": [{"x": 0, "y": 0}, {"x": 20, "y": -20}, {"x": 40, "y": 0}]}))
            .unwrap();
    }
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    for x in [400, 700] {
        let r = app.run("glyph.addComponent", &json!({"glyph": "A", "component": "base"})).unwrap();
        let shape = r["shape"].as_u64().unwrap();
        app.run("component.move", &json!({"glyph": "A", "components": [shape], "dx": x, "dy": 0})).unwrap();
    }
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [450, 250]})).unwrap();
    app.state.display.info = true;
    app
}

#[test]
fn add_component_from_the_menu_opens_the_picker_and_a_pick_gives_the_commands_font() {
    let mut app = app();
    let mut direct = self::app();
    let m = master(&app);
    direct.run("glyph.addComponent", &json!({"glyph": "A", "layer": m, "component": "base"})).unwrap();

    // Escape: the picker closes and nothing changes.
    let before = app.session.font().unwrap().clone();
    let r = app.run_menu_command("glyph.addComponent", json!({})).unwrap();
    assert_eq!((r["open"].clone(), r["purpose"].clone()), (json!(true), json!("addComponent")));
    app.run(PICK, &json!({"open": false})).unwrap();
    assert!(app.session.font().unwrap().same_as(&before));

    // Typing, then Return on the highlighted result.
    app.run_menu_command("glyph.addComponent", json!({})).unwrap();
    let r = app.run(PICK, &json!({"query": "base"})).unwrap();
    assert_eq!(r["selected"], "base", "a new search highlights its first result: {r}");
    assert!(app.run(PICK, &json!({"select": "nosuch"})).is_err(), "only a result can be highlighted");
    app.run(PICK, &json!({"pick": true})).unwrap();
    assert!(!app.state.panels.component_picker.open, "a pick closes the picker");
    assert_eq!(layer(&app, "A"), layer(&direct, "A"));
}

#[test]
fn the_component_info_box_reads_the_selected_component_and_its_name_repoints_it() {
    let mut app = app();
    let mut direct = self::app();
    assert!(info_box_component::read(&app).is_none(), "no box without a selected component");
    app.run("selection.set", &json!({"shapes": [1]})).unwrap();
    let b = info_box_component::read(&app).unwrap();
    assert_eq!((b.base.as_str(), b.component, b.position), ("base", 1, (400.0, 0.0)));
    let v = b.values.unwrap();
    assert_eq!((v.scale_x, v.scale_y, v.slant, v.rotation), (100.0, 100.0, 0.0, 0.0));
    assert_eq!(b.free, (true, true), "a component beside a path is placed by hand");
    assert!(b.anchors.is_empty() && !b.smart);

    // The name opens the base glyph picker on that component.
    let r = app.run(PICK, &json!({"purpose": "baseGlyph", "query": "empty"})).unwrap();
    assert_eq!(r["results"], json!(["empty"]));
    app.run(PICK, &json!({"pick": true})).unwrap();
    let m = master(&direct);
    direct.run("component.setBaseGlyph", &json!({"glyph": "A", "layer": m, "component": 1, "base": "empty"})).unwrap();
    assert_eq!(layer(&app, "A"), layer(&direct, "A"));

    // A frame draws the box with the new base glyph's name.
    let ctx = egui::Context::default();
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    // The first frame measures the boxes; the second draws them in place.
    let _ = ctx.run_ui(input.clone(), |ui| app.ui(ui));
    let out = ctx.run_ui(input, |ui| app.ui(ui));
    let mut shapes: Vec<egui::Shape> = out.shapes.into_iter().map(|c| c.shape).collect();
    let mut texts = Vec::new();
    while let Some(s) = shapes.pop() {
        match s {
            egui::Shape::Vec(list) => shapes.extend(list),
            egui::Shape::Text(t) => texts.push(t.galley.text().to_string()),
            _ => {}
        }
    }
    assert!(texts.iter().any(|t| t == "empty"), "the box names the base glyph: {texts:?}");
}

#[test]
fn info_box_values_split_matrices_without_slant() {
    use fontcraft_model::Transform;
    let turned = info_box_component::values(&Transform::Matrix([0.0, 2.0, -1.0, 0.0, 5.0, 6.0])).unwrap();
    assert_eq!((turned.x, turned.y), (5.0, 6.0));
    assert!((turned.rotation - 90.0).abs() < 1e-9 && (turned.scale_x - 200.0).abs() < 1e-9 && (turned.scale_y - 100.0).abs() < 1e-9);
    let mirrored = info_box_component::values(&Transform::Matrix([-1.0, 0.0, 0.0, 1.0, 0.0, 0.0])).unwrap();
    assert_eq!((mirrored.scale_x, mirrored.scale_y, mirrored.rotation), (-100.0, 100.0, 0.0), "a mirror is a negative scale");
    assert!(info_box_component::values(&Transform::Matrix([1.0, 0.0, 0.5, 1.0, 0.0, 0.0])).is_none(), "a slanted matrix is not split");
    assert!(info_box_component::values(&Transform::Matrix([1.0, 1.0, 1.0, 1.0, 0.0, 0.0])).is_none(), "nor a singular one");
}

#[test]
fn the_anchor_menu_lists_only_anchors_the_mark_can_attach_to() {
    let mut font = Font::new("Test");
    let m = font.masters[0].id.clone();
    let glyph = |name: &str, anchors: &[(&str, f64)], shapes: Vec<Shape>| {
        let mut g = Glyph::new(name);
        let mut l = Layer::new(m.clone(), 500.0);
        l.anchors = anchors.iter().map(|(n, y)| Anchor::new(*n, 100.0, *y)).collect();
        l.shapes = shapes;
        g.layers.push(l);
        g
    };
    font.glyphs.push(glyph("a", &[("top", 500.0), ("top_viet", 600.0), ("bottom", 0.0)], Vec::new()));
    font.glyphs.push(glyph("o", &[("top", 500.0)], Vec::new()));
    font.glyphs.push(glyph("acutecomb", &[("_top", 500.0), ("top", 700.0)], Vec::new()));
    let comp = |base: &str| vec![Shape::Component(Component::new(base, 0.0, 0.0)), Shape::Component(Component::new("acutecomb", 0.0, 0.0))];
    font.glyphs.push(glyph("aacute", &[], comp("a")));
    font.glyphs.push(glyph("oacute", &[], comp("o")));
    let host = |name: &str| font.glyph(name).unwrap().layer(&m).unwrap().clone();
    assert_eq!(info_box_component::anchor_choices(&font, None, &host("aacute"), 1), ["top", "top_viet"], "bottom is no choice for _top");
    assert_eq!(info_box_component::anchor_choices(&font, None, &host("oacute"), 1), ["top"], "one choice: the menu stays hidden");
    assert!(info_box_component::anchor_choices(&font, None, &host("aacute"), 0).is_empty(), "the base attaches to nothing");
}

#[test]
fn component_from_selection_asks_for_a_name_and_gives_the_commands_font() {
    let mut app = app();
    let mut direct = self::app();
    app.run("selection.set", &json!({"shapes": [0]})).unwrap();
    let before = app.session.font().unwrap().clone();

    // Cancel changes nothing.
    app.run_menu_command("glyph.componentFromSelection", json!({})).unwrap();
    let shown = dialogs::inspect(&app);
    assert_eq!(shown["dialog"], "dialog.componentFromSelection");
    assert_eq!(shown["params"]["name"], "_part.A", "the field starts with the suggested name");
    assert_eq!(shown["message"], "The change applies to all master layers.");
    dialogs::cancel(&mut app).unwrap();
    assert!(app.session.font().unwrap().same_as(&before));
    assert_eq!(app.state.tab().unwrap().edited_glyph(), Some("A"));

    // OK with another name.
    app.run_menu_command("glyph.componentFromSelection", json!({})).unwrap();
    dialogs::set(&mut app, "name", &json!("_part.box")).unwrap();
    dialogs::confirm(&mut app).unwrap();
    let m = master(&direct);
    direct.run("glyph.componentFromSelection", &json!({"glyph": "A", "layer": m, "shapes": [0], "name": "_part.box"})).unwrap();
    assert_eq!(layer(&app, "A"), layer(&direct, "A"));
    assert_eq!(layer(&app, "_part.box"), layer(&direct, "_part.box"));
    let tab = app.state.tab().unwrap();
    assert_eq!((tab.edited_glyph(), tab.edited_index()), (Some("_part.box"), Some(1)), "the new glyph follows A in the text and is edited");
}

#[test]
fn the_corner_picker_lists_corner_glyphs_and_the_corner_info_box_follows_the_node() {
    let mut app = app();
    let mut direct = self::app();
    let m = master(&app);
    app.run("selection.set", &json!({"points": [[0, 1]]})).unwrap();
    assert!(info_box_corner::read(&app).is_none(), "no corner on the node yet");
    let r = app.run(PICK, &json!({"purpose": "addCorner"})).unwrap();
    assert_eq!(r["results"], json!(["_corner.serif", "_corner.round"]), "only corner glyphs, all of them while the field is empty");
    let r = app.run(PICK, &json!({"query": "ser"})).unwrap();
    assert_eq!(r["results"], json!(["_corner.serif"]));
    app.run(PICK, &json!({"pick": true})).unwrap();
    direct.run("cornerComponent.add", &json!({"glyph": "A", "layer": m, "points": [[0, 1]], "corner": "_corner.serif"})).unwrap();
    assert_eq!(layer(&app, "A"), layer(&direct, "A"));

    let b = info_box_corner::read(&app).unwrap();
    assert_eq!((b.name.as_str(), b.corner, b.scale, b.mode), ("_corner.serif", 0, (100.0, 100.0), Some("left")));

    // The name opens the picker for the corner on the node; Return replaces its glyph.
    app.run(PICK, &json!({"purpose": "cornerGlyph"})).unwrap();
    app.run(PICK, &json!({"select": "_corner.round", "pick": true})).unwrap();
    direct.run("cornerComponent.setGlyph", &json!({"glyph": "A", "layer": m, "corners": [0], "corner": "_corner.round"})).unwrap();
    assert_eq!(layer(&app, "A"), layer(&direct, "A"));

    // The arrow edits the corner glyph.
    let r = app.run("cornerComponent.editGlyph", &json!({"corner": 0})).unwrap();
    assert_eq!(r["glyph"], "_corner.round");
    assert_eq!(app.state.tab().unwrap().edited_glyph(), Some("_corner.round"));
    assert!(app.run("cornerComponent.editGlyph", &json!({"glyph": "A", "corner": 5})).is_err(), "no such hint");
}

#[test]
fn show_glyphs_using_this_glyph_opens_a_tab_of_its_composites() {
    let mut app = app();
    app.run("glyph.addComponent", &json!({"glyph": "empty", "component": "base"})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["base"]})).unwrap();
    let tabs = app.state.tabs.len();
    let r = app.run("glyph.showGlyphsUsingAsComponent", &json!({})).unwrap();
    let mut glyphs: Vec<String> = serde_json::from_value(r["glyphs"].clone()).unwrap();
    glyphs.sort();
    assert_eq!(glyphs, ["A", "empty"]);
    assert_eq!(app.state.tabs.len(), tabs + 1, "a new tab");
    let before = app.state.tabs.len();
    assert!(app.run("glyph.showGlyphsUsingAsComponent", &json!({"glyph": "A"})).is_err(), "nothing uses A");
    assert_eq!(app.state.tabs.len(), before);
}

#[test]
fn context_menu_items_that_need_a_choice_ask_for_it() {
    let mut app = app();
    // On a component: Change Component Glyph opens the base glyph picker; the anchor is not here.
    context_menu::open(&mut app, Point::new(500.0, 100.0), Space::Font).unwrap();
    let menu = app.tools.context_menu.clone().unwrap();
    assert_eq!(menu.sections[0].context, "canvas.component");
    assert!(menu.item("component.setAnchor").is_none());
    assert!(menu.item("glyph.showGlyphsUsingAsComponent").is_some(), "the canvas section offers the composites of the glyph");
    let r = context_menu::invoke(&mut app, "component.setBaseGlyph").unwrap();
    assert_eq!(r["purpose"], "baseGlyph");
    assert_eq!(app.state.panels.component_picker.targets["component"], 1);
    app.run(PICK, &json!({"open": false})).unwrap();

    // On a node: Add Corner Component opens the corner picker; Decompose Corner only on a corner.
    context_menu::open(&mut app, Point::new(300.0, 300.0), Space::Font).unwrap();
    let menu = app.tools.context_menu.clone().unwrap();
    assert!(menu.item("cornerComponent.decompose").is_none());
    let r = context_menu::invoke(&mut app, "cornerComponent.add").unwrap();
    assert_eq!(r["purpose"], "addCorner");
    app.run(PICK, &json!({"query": "serif", "pick": true})).unwrap();
    context_menu::open(&mut app, Point::new(300.0, 300.0), Space::Font).unwrap();
    let before = app.session.font().unwrap().clone();
    context_menu::invoke(&mut app, "cornerComponent.decompose").unwrap();
    assert!(!app.session.font().unwrap().same_as(&before), "Decompose Corner fits the corner into the path");

    // Add Component opens the component picker; Component from Selection its dialog.
    context_menu::open(&mut app, Point::new(500.0, 100.0), Space::Font).unwrap();
    let r = context_menu::invoke(&mut app, "glyph.addComponent").unwrap();
    assert_eq!(r["purpose"], "addComponent");
    app.run(PICK, &json!({"open": false})).unwrap();
    context_menu::open(&mut app, Point::new(500.0, 100.0), Space::Font).unwrap();
    context_menu::invoke(&mut app, "glyph.componentFromSelection").unwrap();
    assert_eq!(dialogs::inspect(&app)["dialog"], "dialog.componentFromSelection");
    dialogs::cancel(&mut app).unwrap();
}
