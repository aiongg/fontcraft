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
