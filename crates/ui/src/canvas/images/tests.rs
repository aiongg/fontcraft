//! Placed images on the canvas: what is drawn, the selection, the drags and keys, and the decoding limits.

use std::path::PathBuf;
use std::sync::Arc;

use craft::behavior::CanvasView;
use craft::behavior::kurbo::Point as KPoint;
use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use egui::{Pos2, Rect, vec2};
use fontcraft_engine::Session;
use fontcraft_engine::imagery::{self, Rgba8};
use fontcraft_tools::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};
use serde_json::{Value, json};

use super::files::{self, ImageFiles, Loaded};
use super::{Look, is_selected};
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::canvas::tools::{ToolInput, apply};

/// A folder of its own for one test's files.
pub(crate) fn temp_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("fontcraft-ui-images-{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `width × height` PNG of one opaque colour, written to `dir/name`.
pub(crate) fn png(dir: &std::path::Path, name: &str, width: u32, height: u32) -> PathBuf {
    let pixels = [0x30u8, 0x60, 0x90, 0xff].repeat((width * height) as usize);
    let bytes = imagery::to_png(&Rgba8 { width, height, pixels }).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

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

/// Glyph A, empty, with a 100 × 50 pixel image at the origin (one unit per pixel at 72 pixels per inch), shown at one
/// point per unit.
fn image_app(name: &str) -> (FontCraftApp, PathBuf) {
    let dir = temp_dir(name);
    let file = png(&dir, "A.png", 100, 50);
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    app.drain();
    run(&mut app, "glyph.addImage", json!({"files": [file.to_string_lossy()], "glyphs": ["A"]}));
    app.view.canvas = CanvasView::new(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0)), KPoint::new(400.0, 400.0), 1.0);
    app.drain();
    (app, dir)
}

fn image_info(app: &mut FontCraftApp) -> Value {
    app.session.execute("image.getInfo", &json!({"glyph": "A"})).unwrap()["image"].clone()
}

fn undo_steps(app: &FontCraftApp) -> usize {
    app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
}

fn pointer(app: &mut FontCraftApp, kind: PointerKind, x: f64, y: f64, mods: Modifiers) -> Vec<String> {
    let errors = apply::input(app, ToolInput::Pointer(PointerEvent::new(kind, x, y, mods)));
    app.drain();
    errors
}

fn click_drag(app: &mut FontCraftApp, from: (f64, f64), to: (f64, f64), mods: Modifiers) {
    assert!(pointer(app, PointerKind::Down, from.0, from.1, mods).is_empty());
    assert!(pointer(app, PointerKind::Drag, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0, mods).is_empty());
    assert!(pointer(app, PointerKind::Drag, to.0, to.1, mods).is_empty());
    assert!(pointer(app, PointerKind::Up, to.0, to.1, mods).is_empty());
}

/// The image of an empty glyph is drawn whatever Show Image says, at half opacity; a press on it selects it, and a
/// drag moves it as one undo step.
#[test]
fn a_press_selects_the_image_and_a_drag_moves_it_in_one_undo_step() {
    let (mut app, dir) = image_app("drag");
    let placed = &app.tools.view.images.placed;
    assert_eq!(placed.len(), 1);
    assert!(matches!(&placed[0].look, Look::Pixels { size: [100, 50], .. }), "{:?}", placed[0].look);
    assert!((placed[0].alpha - 0.5).abs() < 1e-6);
    shell(&mut app, "view.showImage", json!({"on": false}));
    assert_eq!(app.tools.view.images.placed.len(), 1, "an empty glyph shows its image with Show Image off");

    let before = undo_steps(&app);
    click_drag(&mut app, (20.0, 20.0), (60.0, 40.0), Modifiers::NONE);
    assert!(is_selected(&app) && app.tools.selection.is_empty());
    assert!(app.tools.view.images.selected.is_some());
    let info = image_info(&mut app);
    assert_eq!((info["x"].as_f64(), info["y"].as_f64()), (Some(40.0), Some(20.0)), "{info}");
    assert_eq!(undo_steps(&app), before + 1);

    // A press beside the image deselects it.
    pointer(&mut app, PointerKind::Down, 500.0, 500.0, Modifiers::NONE);
    pointer(&mut app, PointerKind::Up, 500.0, 500.0, Modifiers::NONE);
    assert!(!is_selected(&app));
    let _ = std::fs::remove_dir_all(dir);
}

/// A locked image takes no press; behind paths the image follows Show Image.
#[test]
fn a_locked_image_is_not_selected_and_show_image_hides_it_behind_paths() {
    let (mut app, dir) = image_app("locked");
    run(&mut app, "image.lock", json!({"glyph": "A"}));
    pointer(&mut app, PointerKind::Down, 20.0, 20.0, Modifiers::NONE);
    pointer(&mut app, PointerKind::Up, 20.0, 20.0, Modifiers::NONE);
    assert!(!is_selected(&app));

    run(&mut app, "path.addRectangle", json!({"glyph": "A", "x": 300, "y": 300, "width": 100, "height": 100}));
    assert_eq!(app.tools.view.images.placed.len(), 1);
    shell(&mut app, "view.showImage", json!({"on": false}));
    assert!(app.tools.view.images.placed.is_empty());
    assert!(crate::app::menu_states::checks(&app).contains(&("view.showImage", false)));
    let _ = std::fs::remove_dir_all(dir);
}

/// The arrows move the selected image by the font's increments and Delete removes it; menu commands that take the
/// image get it.
#[test]
fn arrows_move_the_selected_image_and_delete_removes_it() {
    let (mut app, dir) = image_app("keys");
    pointer(&mut app, PointerKind::Down, 20.0, 20.0, Modifiers::NONE);
    pointer(&mut app, PointerKind::Up, 20.0, 20.0, Modifiers::NONE);
    assert!(is_selected(&app));
    let shift = Modifiers { shift: true, ..Modifiers::NONE };
    assert!(apply::input(&mut app, ToolInput::Key(KeyEvent::press(Key::Right, shift))).is_empty());
    app.drain();
    assert_eq!(image_info(&mut app)["x"].as_f64(), Some(10.0));

    let moved = crate::canvas::tools::targets::with_targets(&mut app, CommandRef::new("selection.move"));
    assert_eq!(moved.params["image"], json!(true), "{}", moved.params);

    assert!(apply::input(&mut app, ToolInput::Key(KeyEvent::press(Key::Delete, Modifiers::NONE))).is_empty());
    app.drain();
    assert!(image_info(&mut app).is_null());
    assert!(app.tools.view.images.placed.is_empty() && !app.tools.image);
    let _ = std::fs::remove_dir_all(dir);
}

/// With only the placed image selected, no menu row of a command that takes the image is greyed out.
#[test]
fn a_selected_image_supplies_the_menu_rows_of_the_commands_that_take_it() {
    let (mut app, dir) = image_app("menu-rows");
    pointer(&mut app, PointerKind::Down, 20.0, 20.0, Modifiers::NONE);
    pointer(&mut app, PointerKind::Up, 20.0, 20.0, Modifiers::NONE);
    app.drain();
    assert!(is_selected(&app) && app.tools.selection.is_empty());
    let greyed = crate::app::menu_states::greyed(&app);
    let takes_image: Vec<&str> = greyed
        .iter()
        .filter(|(id, _)| fontcraft_engine::find_command(id).is_some_and(|spec| crate::canvas::tools::targets::takes_param(spec, "image?: bool")))
        .map(|(id, _)| *id)
        .collect();
    assert!(takes_image.is_empty(), "greyed although the image supplies them: {takes_image:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A file whose header states more pixels than are drawn is refused from its header; a missing file is the marker.
#[test]
fn files_past_the_limits_are_refused_without_decoding() {
    let dir = temp_dir("limits");
    // A PNG signature and an IHDR chunk stating 20000 × 20000 pixels: nothing after it is read.
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&20_000u32.to_be_bytes());
    bytes.extend_from_slice(&20_000u32.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    let huge = dir.join("huge.png");
    std::fs::write(&huge, bytes).unwrap();
    let session = Session::with_document();
    let assets = session.assets();
    let mut cache = ImageFiles::default();
    let got = cache.get(&assets, &huge.to_string_lossy(), None, None);
    assert!(matches!(got, Loaded::Failed(_) | Loaded::Unsupported { .. }), "{got:?}");
    let missing = cache.get(&assets, &dir.join("gone.png").to_string_lossy(), None, None);
    assert!(matches!(missing, Loaded::Failed(_)));
    let small = png(&dir, "small.png", 8, 4);
    assert!(matches!(cache.get(&assets, &small.to_string_lossy(), None, None), Loaded::Ready { size: [8, 4], .. }));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn shrinking_averages_whole_blocks() {
    let image = Rgba8 { width: 4, height: 2, pixels: [[0u8, 0, 0, 255], [255, 255, 255, 255]].concat().repeat(4) };
    let small = files::shrink(image, 2);
    assert_eq!((small.width, small.height), (2, 1));
    assert_eq!(&small.pixels[..4], &[127, 127, 127, 255]);
}

/// A right-click on an unlocked image selects it and offers the crop items, and the Info box shows its part; on a
/// locked image the menu offers Unlock Image, which unlocks it.
#[test]
fn the_context_menu_on_the_image_offers_its_items_and_the_info_box_shows_it() {
    use crate::app::context_menu::{self as menus, Space};
    let (mut app, dir) = image_app("menu");
    let items = |app: &FontCraftApp| -> (String, Vec<&'static str>) {
        let menu = app.context_menus.canvas.clone().expect("the canvas menu");
        let first = menu.sections.first().expect("a section");
        (first.context.to_string(), first.items.iter().map(|i| i.command).collect())
    };
    menus::open_canvas(&mut app, fontcraft_geom::Point::new(20.0, 20.0), Space::Font).unwrap();
    assert_eq!(items(&app), ("canvas.image".to_string(), vec!["image.setCropToLayerBounds", "image.resetCrop"]));
    assert!(is_selected(&app));
    app.drain();
    let part = app.models.palette.info_box.image.clone().expect("the image part of the Info box");
    assert_eq!((part.x, part.width, part.height), (Some(0.0), Some(100.0), Some(50.0)));

    run(&mut app, "image.lock", json!({"glyph": "A"}));
    assert!(app.models.palette.info_box.image.is_none(), "a locked image is not selected");
    menus::open_canvas(&mut app, fontcraft_geom::Point::new(20.0, 20.0), Space::Font).unwrap();
    assert_eq!(items(&app).1, vec!["image.unlock"]);
    menus::invoke(&mut app, "image.unlock", |app, msg| app.run_now(msg, false)).unwrap();
    assert_eq!(image_info(&mut app)["locked"], json!(false));
    let _ = std::fs::remove_dir_all(dir);
}

/// Reveal Image File shows the edited image's file through the window's opener; without a file manager it says so.
#[test]
fn reveal_image_file_goes_through_the_opener() {
    let (mut app, dir) = image_app("reveal");
    app.sent.push(Msg::Shell(CommandRef::with_params("image.revealFile", json!({}))));
    app.drain();
    assert!(!app.status.is_empty(), "tests have no file manager: {}", app.status);
    app.status.clear();
    app.sent.push(Msg::Shell(CommandRef::with_params("image.revealFile", json!({"glyph": "B"}))));
    app.drain();
    assert!(app.status.contains("no image") || app.status.contains("no glyph") || app.status.contains("no layer"), "{}", app.status);
    let _ = std::fs::remove_dir_all(dir);
}

/// One file dropped on the canvas goes on the edited layer's edited side; several go on the glyphs their names match.
#[test]
fn dropped_files_place_as_add_image_does() {
    use super::drop::{canvas_params, cell_params};
    let (mut app, dir) = image_app("drop");
    let one = vec![dir.join("x.png")];
    let master = app.session.font().unwrap().masters[0].id.clone();
    let layer = crate::canvas::tools::target(&app.session, &app.view).unwrap().layer;
    assert_eq!(canvas_params(&app, &one), Some(json!({"files": [dir.join("x.png").to_string_lossy()], "glyphs": ["A"], "layer": layer})));
    shell(&mut app, "path.editBackground", json!({"on": true}));
    assert_eq!(canvas_params(&app, &one).unwrap()["side"], json!("background"));
    let two = vec![dir.join("A.png"), dir.join("B.jpeg"), dir.join("notes.txt")];
    let params = canvas_params(&app, &two).unwrap();
    assert_eq!(params["files"].as_array().map(Vec::len), Some(2));
    assert_eq!(params["layer"], json!(master));
    assert_eq!(cell_params("C", &master, &one), Some(json!({"files": [dir.join("x.png").to_string_lossy()], "glyphs": ["C"], "layer": master})));
    assert_eq!(cell_params("C", &master, &[dir.join("notes.txt")]), None);
    let _ = std::fs::remove_dir_all(dir);
}

/// Font View cells draw the image of their glyph's master layer: always on an empty glyph, behind paths while Show
/// Image is on.
#[test]
fn font_view_cells_draw_placed_images() {
    let (mut app, dir) = image_app("cells");
    shell(&mut app, "fontView.show", json!({}));
    assert!(app.font_view_shows());
    let cell = app.models.cell_images.by_glyph.get("A").cloned().expect("A's cell image");
    assert!(matches!(cell.look, Look::Pixels { size: [100, 50], .. }), "{:?}", cell.look);
    run(&mut app, "path.addRectangle", json!({"glyph": "A", "x": 300, "y": 300, "width": 100, "height": 100}));
    assert!(app.models.cell_images.by_glyph.contains_key("A"));
    shell(&mut app, "view.showImage", json!({"on": false}));
    assert!(!app.models.cell_images.by_glyph.contains_key("A"));
    let _ = std::fs::remove_dir_all(dir);
}
