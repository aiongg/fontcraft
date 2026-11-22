//! The Text Preview through the app layer: each command changes what the projection places, characters the font
//! lacks are placed for the interface font, the text limit, shaping through the preview font, and the rebuild once
//! editing pauses, and colour glyphs.

use std::sync::Arc;

use craft::host::{Host, InlineJobs};
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::prelude::*;
use serde_json::{Value, json};

use crate::app::models::text_preview::{Page, PageDirection, fail_color_font, set_now};
use crate::app::{APP, FontCraftApp, Msg};

/// A new font whose preview fonts build at once, on the calling thread; the Text Preview open.
fn app() -> FontCraftApp {
    let host = Host::builder(APP).jobs(Arc::new(InlineJobs)).build();
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(host));
    for (glyph, x) in [("A", 50), ("V", 100)] {
        exec(&mut app, "path.addRectangle", json!({"glyph": glyph, "x": x, "y": 0, "width": 400, "height": 700}));
    }
    exec(&mut app, "kerning.setPair", json!({"left": "A", "right": "V", "value": -100}));
    shell(&mut app, "window.textPreview", json!({})).unwrap();
    app
}

fn exec(app: &mut FontCraftApp, id: &str, params: Value) -> Value {
    app.run_now(Msg::run(id, params), false).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn shell(app: &mut FontCraftApp, id: &str, params: Value) -> Result<Value, String> {
    app.run_now(Msg::Shell(CommandRef::with_params(id, params)), false)
}

fn page(app: &FontCraftApp) -> Arc<Page> {
    app.models.text_preview.view.page.clone().expect("a placed page")
}

fn names(p: &Page) -> Vec<&str> {
    p.glyphs.iter().map(|g| &*g.name).collect()
}

fn xs(p: &Page) -> Vec<f64> {
    p.glyphs.iter().map(|g| g.x).collect()
}

fn width(app: &FontCraftApp, glyph: &str) -> f64 {
    app.session.font().unwrap().glyph(glyph).unwrap().layers[0].width
}

/// Text, size, orientation, writing direction and find each change what the preview places.
#[test]
fn text_preview_commands_change_what_the_projection_places() {
    let mut app = app();
    assert!(app.models.text_preview.view.font, "a font is open");
    assert!(page(&app).glyphs.is_empty(), "an empty text places nothing");

    shell(&mut app, "textPreview.setText", json!({"text": "AVA"})).unwrap();
    let p = page(&app);
    assert_eq!(names(&p), ["A", "V", "A"]);
    let x = xs(&p);
    assert!((x[1] - x[0] - (width(&app, "A") - 100.0)).abs() < 0.01, "A V kerned by −100: {x:?}");
    assert!(p.outlines.paths.contains_key("A") && p.outlines.paths.contains_key("V"), "the outlines of what is placed");

    let scale = p.scale;
    shell(&mut app, "textPreview.setFontSize", json!({"size": 72})).unwrap();
    assert!((page(&app).scale - 2.0 * scale).abs() < 1e-9);
    assert!(shell(&mut app, "textPreview.setFontSize", json!({"size": 2})).is_err(), "a size below 4 points is refused");

    shell(&mut app, "textPreview.setWritingDirection", json!({"scope": "text", "direction": "rtl"})).unwrap();
    let p = page(&app);
    assert_eq!(p.direction, PageDirection::Rtl);
    let x = xs(&p);
    assert!(x.iter().all(|x| *x < 0.0), "the line runs from the right edge: {x:?}");
    assert!(x[0] < x[1] && x[1] < x[2], "a Latin run still reads left to right, as bidi orders it: {x:?}");

    shell(&mut app, "textPreview.setLayoutOrientation", json!({"orientation": "vertical"})).unwrap();
    assert_eq!(page(&app).direction, PageDirection::Vertical);

    shell(&mut app, "textPreview.find", json!({"query": "a"})).unwrap();
    let model = &app.models.text_preview.view;
    assert_eq!((model.matches, model.find.current), (2, Some(0)));
    let current = page(&app).highlights.iter().filter(|b| b.current).count();
    assert_eq!(current, 1, "the current match is highlighted");
    shell(&mut app, "textPreview.find", json!({"replace": "V"})).unwrap();
    assert_eq!(&*app.models.text_preview.view.text, "VVV");
    assert_eq!(app.view.text_preview.find.replaced, 2);

    shell(&mut app, "window.textPreview", json!({"open": false})).unwrap();
    let mut state = serde_json::Map::new();
    super::inspect(&app, &mut state);
    assert_eq!(state["textPreview"]["open"], false);
}

/// A character the font lacks is placed between its neighbours for the interface font.
#[test]
fn text_preview_places_a_missing_character_for_the_interface_font() {
    let mut app = app();
    shell(&mut app, "textPreview.setText", json!({"text": "A\u{e9}A"})).unwrap();
    let p = page(&app);
    assert_eq!(names(&p), ["A", "A"]);
    assert_eq!(p.fallback.len(), 1);
    let c = &p.fallback[0];
    assert_eq!(c.c, '\u{e9}');
    assert!(p.glyphs[0].x < c.x && c.x < p.glyphs[1].x, "between the two A: {p:?}");
}

/// A text past the limit is refused and changes nothing; a long text places only the lines around the view.
#[test]
fn text_preview_refuses_more_than_its_limit_and_places_only_the_lines_around_the_view() {
    let mut app = app();
    let long = "AV ".repeat(100_000);
    shell(&mut app, "textPreview.setText", json!({"text": long})).unwrap();
    let p = page(&app);
    assert!(p.lines > 100, "{} lines", p.lines);
    assert!(p.placed.end < p.lines, "only the lines around the view are placed: {:?} of {}", p.placed, p.lines);
    let too_long = "A".repeat(super::MAX_CHARS + 1);
    assert!(shell(&mut app, "textPreview.setText", json!({"text": too_long})).is_err());
    assert_eq!(app.view.text_preview.length, 300_000, "the refused text changed nothing");
}

/// The text is shaped through the compiled preview font, whose kerning places V; an edit builds the font again only
/// once editing has paused.
#[test]
fn text_preview_shapes_through_the_preview_font_and_rebuilds_once_editing_pauses() {
    let mut app = app();
    shell(&mut app, "textPreview.setText", json!({"text": "AV"})).unwrap();
    let p = page(&app);
    assert_eq!(p.shaped, 1, "the line is shaped: {:?}", app.models.text_preview.view.message);
    let x = xs(&p);
    assert!((x[1] - x[0] - (width(&app, "A") - 100.0)).abs() < 0.01, "kerned by the compiled GPOS: {x:?}");

    exec(&mut app, "spacing.setWidth", json!({"glyphs": ["A"], "value": 700}));
    let doc = app.session.active().map(|d| (d.id, d.revision)).unwrap();
    let master = app.session.font().unwrap().masters[0].id.clone();
    assert!(app.session.preview_builds().get(doc.0, doc.1, &master).is_none(), "no build while editing");
    set_now(&mut app.models.text_preview, 1.0);
    app.drain();
    assert!(app.session.preview_builds().get(doc.0, doc.1, &master).is_some(), "built once editing paused");
    let x = xs(&page(&app));
    assert!((x[1] - x[0] - 600.0).abs() < 0.01, "the new width, kerned: {x:?}");
}

/// A colour glyph draws from the compiled colour font: palette row 0 of `CPAL` fills A's square, V (no colour) draws
/// its outline. When the colour font cannot be built, every glyph is an outline and the reason shows under the text.
#[test]
fn text_preview_draws_colour_glyphs_from_the_compiled_font() {
    let mut app = app();
    exec(&mut app, "colorPalette.addColor", json!({"color": "#FF0000"}));
    let master = app.session.font().unwrap().masters[0].id.clone();
    exec(&mut app, "layers.addBackupLayer", json!({"glyphs": ["A"], "master": master}));
    let layer = app.session.font().unwrap().glyph("A").unwrap().layers.iter().find(|l| !l.is_master_layer()).unwrap().id.clone();
    exec(&mut app, "layer.convertToColorPalette", json!({"glyph": "A", "layer": layer}));
    shell(&mut app, "textPreview.setText", json!({"text": "AV"})).unwrap();
    // The colour font of the edited document builds once editing has paused.
    set_now(&mut app.models.text_preview, 1.0);
    app.drain();
    let p = page(&app);
    let Some(Some(a)) = p.colors.rasters.get("A") else { panic!("A draws in colour: {:?}", app.models.text_preview.view.message) };
    assert!((p.colors.pixel_scale - p.scale).abs() < 1e-9, "drawn at one pixel per point until the view reports its own");
    let red = a.image.pixels.as_chunks::<4>().0.iter().filter(|px| **px == [255, 0, 0, 255]).count();
    assert!(red > 0, "palette row 0 fills the square");
    assert!(matches!(p.colors.rasters.get("V"), Some(None)), "V draws its outline");
    assert_eq!(app.models.text_preview.view.message, None);

    let reason = "The colour glyphs are drawn as outlines: the font could not be built";
    fail_color_font(&mut app.models.text_preview, reason);
    app.drain();
    let p = page(&app);
    assert!(p.colors.rasters.values().all(Option::is_none), "every glyph is an outline");
    assert_eq!(p.glyphs.len(), 2);
    assert_eq!(app.models.text_preview.view.message.as_deref(), Some(reason), "the reason shows under the text");
}

/// The waterfall repeats the Edit View text once per size, in one page for the master it draws; sizes outside the
/// allowed range and too many sizes are refused, and turning it off brings the running text back.
#[test]
fn the_waterfall_repeats_the_edit_view_text_at_each_size() {
    let mut app = app();
    shell(&mut app, "editView.setText", json!({"text": "AV"})).unwrap();
    shell(&mut app, "view.previewWaterfall", json!({"on": true, "sizes": [72, 12]})).unwrap();
    let model = &app.models.text_preview.view;
    assert_eq!(model.waterfall.iter().map(|r| r.size).collect::<Vec<_>>(), [72.0, 12.0]);
    assert!(model.waterfall.iter().all(|r| r.optical_size.is_none() && r.page == 0), "no optical-size axis: one master");
    assert_eq!(model.waterfall_pages.len(), 1);
    assert_eq!(names(&model.waterfall_pages[0]), ["A", "V"]);
    let x = xs(&model.waterfall_pages[0]);
    assert!((x[1] - x[0] - (width(&app, "A") - 100.0)).abs() < 0.01, "the line is kerned: {x:?}");

    assert!(shell(&mut app, "view.previewWaterfall", json!({"sizes": [2]})).is_err(), "below 4 points");
    assert!(shell(&mut app, "view.previewWaterfall", json!({"sizes": vec![12; 33]})).is_err(), "more than 32 sizes");
    assert_eq!(app.view.text_preview.waterfall.sizes, [72.0, 12.0], "a refused change keeps the sizes");
    let mut state = serde_json::Map::new();
    super::inspect(&app, &mut state);
    assert_eq!(state["textPreview"]["waterfall"]["lines"].as_array().map(Vec::len), Some(2));

    shell(&mut app, "view.previewWaterfall", json!({})).unwrap();
    assert!(app.models.text_preview.view.waterfall.is_empty(), "without on, the waterfall toggles off");
    shell(&mut app, "textPreview.setText", json!({"text": "AVA"})).unwrap();
    assert_eq!(names(&page(&app)), ["A", "V", "A"]);
}

/// Insert Pattern puts a token into the search, at the end or at a given place, and the search runs again.
#[test]
fn insert_pattern_puts_a_token_into_the_search() {
    let mut app = app();
    shell(&mut app, "textPreview.setText", json!({"text": "A1 V2 AV"})).unwrap();
    shell(&mut app, "textPreview.insertPattern", json!({"pattern": "letter"})).unwrap();
    shell(&mut app, "textPreview.insertPattern", json!({"pattern": "digit"})).unwrap();
    let model = &app.models.text_preview.view;
    assert_eq!((model.find.query.as_str(), model.find.open, model.matches), ("\\l\\d", true, 2));
    shell(&mut app, "textPreview.insertPattern", json!({"pattern": "space", "at": 0})).unwrap();
    let model = &app.models.text_preview.view;
    assert_eq!((model.find.query.as_str(), model.matches), ("\\s\\l\\d", 1));
    let mut state = serde_json::Map::new();
    super::inspect(&app, &mut state);
    assert_eq!(state["textPreview"]["find"]["ranges"], json!([[2, 5]]), "a match spans one character per token");
    assert!(shell(&mut app, "textPreview.insertPattern", json!({"pattern": "word"})).is_err());
    assert!(shell(&mut app, "textPreview.insertPattern", json!({"pattern": "digit", "at": 99})).is_err());
}

/// The preview draws the fonts the last export wrote and the font files of a folder, from the files' own tables, and
/// reads a file again once it changes on disk; the document source brings the open document back.
#[test]
fn the_preview_shows_the_last_export_and_a_folder_and_follows_changes() {
    let dir = std::env::temp_dir().join(format!("fontcraft-ui-text-preview-exported-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = app();
    shell(&mut app, "textPreview.setText", json!({"text": "AV"})).unwrap();
    assert!(shell(&mut app, "textPreview.showExportedFonts", json!({"source": "folder"})).is_err(), "a folder source needs its folder");
    assert!(shell(&mut app, "textPreview.showExportedFonts", json!({"source": "disk"})).is_err());

    shell(&mut app, "textPreview.showExportedFonts", json!({"source": "lastExport"})).unwrap();
    assert!(app.models.text_preview.view.page.is_none(), "nothing exported yet");
    assert!(app.models.text_preview.view.message.is_some(), "the preview says why");

    let out = dir.join("Exported.ttf");
    exec(&mut app, "font.compile", json!({"formats": ["ttf"], "destination": out.to_string_lossy()}));
    assert_eq!(app.view.text_preview.last_export, [out.clone()], "the export's file is kept as the last export");
    set_now(&mut app.models.text_preview, 10.0);
    app.drain();
    let model = &app.models.text_preview.view;
    assert_eq!(model.exported.iter().map(|r| (r.file.as_str(), r.shown)).collect::<Vec<_>>(), [("Exported.ttf", true)]);
    let p = page(&app);
    assert_eq!(names(&p), ["A", "V"], "laid out from the compiled file");
    let before = xs(&p);

    shell(&mut app, "textPreview.showExportedFonts", json!({"source": "folder", "folder": dir.to_string_lossy()})).unwrap();
    set_now(&mut app.models.text_preview, 20.0);
    app.drain();
    assert_eq!(names(&page(&app)), ["A", "V"], "the folder's font file");

    // A wider A, exported again over the same file, shows once the next look at the disk comes round.
    exec(&mut app, "spacing.setWidth", json!({"glyphs": ["A"], "value": 900}));
    exec(&mut app, "font.compile", json!({"formats": ["ttf"], "destination": out.to_string_lossy()}));
    set_now(&mut app.models.text_preview, 22.0);
    app.drain();
    let after = xs(&page(&app));
    assert!(after[1] - after[0] > before[1] - before[0], "the changed file is read again: {before:?} then {after:?}");

    shell(&mut app, "textPreview.showExportedFonts", json!({"source": "document"})).unwrap();
    assert!(app.models.text_preview.view.exported.is_empty());
    assert_eq!(names(&page(&app)), ["A", "V"], "the open document again");
    let _ = std::fs::remove_dir_all(&dir);
}
