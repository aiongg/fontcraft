//! Colour palette commands: list, add and set colours, add and remove palettes, remove a colour
//! with the renumbering of the layers that use later rows, and a master's own palettes.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::{EngineError, Session};
use fontcraft_model::{Background, Color, Glyph, Layer, PaletteEntry, Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Runs a command that must be refused with a message holding `needle`, and checks that the
/// font is the very same one.
fn refused(s: &mut Session, id: &str, p: Value, needle: &str) {
    let before = Arc::clone(&s.doc().unwrap().font);
    match s.execute(id, &p) {
        Err(EngineError::BadParams { message, .. }) => assert!(message.contains(needle), "{id} {p}: {message}"),
        other => panic!("{id} {p}: expected a refusal naming `{needle}`, got {other:?}"),
    }
    assert!(Arc::ptr_eq(&before, &s.doc().unwrap().font), "{id} {p}: a refused command changed the font");
}

fn palettes(s: &mut Session, p: Value) -> Value {
    run(s, "colorPalette.list", p)
}

/// A palette layer of `master` using `entry`, with one path whose fill is palette row `fill`.
fn palette_layer(master: &str, id: &str, entry: PaletteEntry, fill: Option<u16>) -> Layer {
    let mut l = Layer::new(master, 500.0);
    l.id = id.into();
    l.info.attrs.color_palette = Some(entry);
    if let Some(index) = fill {
        let mut path = Path::new(true, Vec::new());
        path.info.attrs.fill_color = Some(Color::Palette { index, alpha: 1.0 });
        l.shapes.push(Shape::Path(path));
    }
    l
}

/// Three colours in two palettes, and glyphs whose layers use rows 0, 2 and the text colour.
fn palette_font() -> Session {
    let mut s = Session::with_document();
    for c in ["#FF0000", "#00FF00", "#0000FF"] {
        run(&mut s, "colorPalette.addColor", json!({"color": c}));
    }
    run(&mut s, "colorPalette.addPalette", json!({}));
    s.edit("glyphs", |font| {
        let master = font.masters[0].id.clone();
        let mut a = Glyph::new("heart");
        a.layers.push(Layer::new(master.as_str(), 500.0));
        a.layers.push(palette_layer(&master, "a0", PaletteEntry::Index(0), None));
        a.layers.push(palette_layer(&master, "a2", PaletteEntry::Index(2), Some(2)));
        a.layers.push(palette_layer(&master, "a*", PaletteEntry::Foreground, None));
        font.glyphs.push(a);
        let mut b = Glyph::new("star");
        b.layers.push(Layer::new(master.as_str(), 500.0));
        b.layers.push(palette_layer(&master, "b1", PaletteEntry::Index(1), None));
        font.glyphs.push(b);
        Ok(())
    })
    .unwrap();
    s
}

fn entry(s: &Session, glyph: &str, layer: &str) -> Option<PaletteEntry> {
    let font = s.font().unwrap();
    font.glyph(glyph).unwrap().layers.iter().find(|l| l.id == layer).unwrap().info.attrs.color_palette
}

#[test]
fn palette_commands_edit_the_font_palettes_and_keep_every_palette_the_same_length() {
    let mut s = Session::with_document();
    assert_eq!(palettes(&mut s, json!({})), json!({"master": null, "source": null, "rows": 0, "palettes": []}));
    // colorPalette.addColor: the first colour makes the first palette; the default is black.
    assert_eq!(run(&mut s, "colorPalette.addColor", json!({"color": "#ff0000"}))["index"], json!(0));
    run(&mut s, "colorPalette.addColor", json!({}));
    // colorPalette.addPalette copies palette 0.
    let r = run(&mut s, "colorPalette.addPalette", json!({}));
    assert_eq!((r["palette"].as_u64(), r["palettes"].as_u64()), (Some(1), Some(2)));
    // One colour each with `colors`.
    run(&mut s, "colorPalette.addColor", json!({"colors": ["#00FF0080", "#0000FF"]}));
    // colorPalette.setColor changes one palette only.
    let r = run(&mut s, "colorPalette.setColor", json!({"palette": 1, "index": 0, "color": "#FFFF00"}));
    assert_eq!(r["color"], json!("#FFFF00FF"));
    let list = palettes(&mut s, json!({}));
    assert_eq!(list["source"], json!("font"));
    assert_eq!(list["rows"], json!(3));
    assert_eq!(list["palettes"], json!([["#FF0000FF", "#000000FF", "#00FF0080"], ["#FFFF00FF", "#000000FF", "#0000FFFF"]]));
    // Stored as the version 3 form: lists of integers.
    let stored = s.font().unwrap().info.params.get("Color Palettes").unwrap().clone();
    let first = &stored.as_array().unwrap().items[0].as_array().unwrap().items[0];
    assert_eq!(first.as_array().unwrap().items.len(), 4);
    // colorPalette.removePalette; the last palette stays.
    let r = run(&mut s, "colorPalette.removePalette", json!({"palette": 0}));
    assert_eq!(r["palettes"], json!(1));
    assert_eq!(palettes(&mut s, json!({}))["palettes"], json!([["#FFFF00FF", "#000000FF", "#0000FFFF"]]));
    refused(&mut s, "colorPalette.removePalette", json!({"palette": 0}), "last palette");
    // Each edit is one undo step.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(palettes(&mut s, json!({}))["palettes"].as_array().unwrap().len(), 2);
    // Refusals change nothing.
    refused(&mut s, "colorPalette.setColor", json!({"index": 0, "color": "red"}), "#RRGGBB");
    refused(&mut s, "colorPalette.setColor", json!({"palette": 5, "index": 0, "color": "#FFFFFF"}), "palette 5");
    refused(&mut s, "colorPalette.setColor", json!({"index": 9, "color": "#FFFFFF"}), "colour 9");
    refused(&mut s, "colorPalette.addColor", json!({"colors": ["#FFFFFF"]}), "2 palettes");
    refused(&mut s, "colorPalette.addColor", json!({"color": "#FFFFFF", "colors": ["#FFFFFF", "#FFFFFF"]}), "not both");
    refused(&mut s, "colorPalette.addPalette", json!({"copyFrom": 7}), "palette 7");
    refused(&mut s, "colorPalette.removeColor", json!({"index": 3}), "colour 3");
}

#[test]
fn removing_a_colour_renumbers_later_rows_and_refuses_a_used_row() {
    let mut s = palette_font();
    // Row 1 is used by star; row 0 by heart.
    refused(&mut s, "colorPalette.removeColor", json!({"index": 1}), "used by star");
    refused(&mut s, "colorPalette.removeColor", json!({"index": 0}), "used by heart");
    // Free row 1, then remove it: heart's row 2 (layer and fill) becomes row 1, in one undo step.
    s.edit("free row 1", |font| {
        let mut b = font.glyph_mut("star")?;
        b.layers.retain(|l| l.id != "b1");
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "colorPalette.removeColor", json!({"index": 1}));
    assert_eq!(r["renumbered"], json!(["heart"]));
    assert_eq!(r["rows"], json!(2));
    assert_eq!(entry(&s, "heart", "a2"), Some(PaletteEntry::Index(1)));
    assert_eq!(entry(&s, "heart", "a0"), Some(PaletteEntry::Index(0)));
    assert_eq!(entry(&s, "heart", "a*"), Some(PaletteEntry::Foreground));
    let fill = {
        let font = s.font().unwrap();
        let layer = font.glyph("heart").unwrap().layers.iter().find(|l| l.id == "a2").unwrap().clone();
        let Some(Shape::Path(p)) = layer.shapes.first() else { panic!("no path") };
        p.info.attrs.fill_color
    };
    assert_eq!(fill, Some(Color::Palette { index: 1, alpha: 1.0 }));
    // Every palette lost the row: blue moved to row 1.
    assert_eq!(palettes(&mut s, json!({}))["palettes"], json!([["#FF0000FF", "#0000FFFF"], ["#FF0000FF", "#0000FFFF"]]));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(entry(&s, "heart", "a2"), Some(PaletteEntry::Index(2)));
    assert_eq!(palettes(&mut s, json!({}))["rows"], json!(3));
}

/// The fill of the first background path of `layer` in `glyph`.
fn background_fill(s: &Session, glyph: &str, layer: &str) -> Option<Color> {
    let font = s.font().unwrap();
    let layer = font.glyph(glyph).unwrap().layers.iter().find(|l| l.id == layer).unwrap().clone();
    let Some(Shape::Path(p)) = layer.info.background.as_ref().and_then(|b| b.shapes.first()) else { panic!("no background path") };
    p.info.attrs.fill_color
}

#[test]
fn background_shapes_hold_and_follow_their_palette_rows() {
    let mut s = palette_font();
    let set_background_row = |s: &mut Session, index: u16| {
        s.edit("background", |font| {
            {
                let mut g = font.glyph_mut("heart")?;
                let layer = g.layers.iter_mut().find(|l| l.id == "a0").unwrap();
                let mut path = Path::new(true, Vec::new());
                path.info.attrs.fill_color = Some(Color::Palette { index, alpha: 1.0 });
                layer.info.background = Some(Background { shapes: vec![Shape::Path(path)], ..Background::default() });
            }
            let mut b = font.glyph_mut("star")?;
            b.layers.retain(|l| l.id != "b1");
            Ok(())
        })
        .unwrap();
    };
    // Only heart's background uses row 1: it still holds the row.
    set_background_row(&mut s, 1);
    refused(&mut s, "colorPalette.removeColor", json!({"index": 1}), "used by heart");
    // A background colour after the removed row moves down with the foreground's.
    set_background_row(&mut s, 2);
    run(&mut s, "colorPalette.removeColor", json!({"index": 1}));
    assert_eq!(background_fill(&s, "heart", "a0"), Some(Color::Palette { index: 1, alpha: 1.0 }));
    assert_eq!(entry(&s, "heart", "a2"), Some(PaletteEntry::Index(1)));
}

#[test]
fn a_master_s_own_palettes_win_and_are_edited_on_their_own() {
    let mut s = palette_font();
    let master = s.font().unwrap().masters[0].id.clone();
    assert_eq!(palettes(&mut s, json!({"master": master}))["source"], json!("font"));
    // An edit with `master` gives the master its own palettes, made from the font's.
    run(&mut s, "colorPalette.setColor", json!({"master": master, "index": 0, "color": "#123456"}));
    let own = palettes(&mut s, json!({"master": master}));
    assert_eq!(own["source"], json!("master"));
    assert_eq!(own["palettes"][0][0], json!("#123456FF"));
    assert_eq!(palettes(&mut s, json!({}))["palettes"][0][0], json!("#FF0000FF"));
    // The font's palettes no longer apply to that master's layers, so star's use of row 1 does not
    // hold the font's row 1, and heart's layers are not renumbered by an edit of the font's.
    let r = run(&mut s, "colorPalette.removeColor", json!({"index": 1}));
    assert_eq!(r["renumbered"], json!([]));
    assert_eq!(entry(&s, "heart", "a2"), Some(PaletteEntry::Index(2)));
    // The master's own row 1 is still in use.
    refused(&mut s, "colorPalette.removeColor", json!({"master": master, "index": 1}), "used by star");
    refused(&mut s, "colorPalette.list", json!({"master": "nope"}), "no master");
}

#[test]
fn unreadable_stored_palettes_refuse_edits() {
    let mut s = Session::with_document();
    s.edit("ragged", |font| {
        let ragged = fontcraft_model::plist::openstep::parse("(((255,0,0,255),(0,0,0,255)),((1,2,3,4)))").unwrap().root;
        Arc::make_mut(&mut font.info).params.set("Color Palettes", ragged);
        Ok(())
    })
    .unwrap();
    refused(&mut s, "colorPalette.list", json!({}), "palette 1 has 1 colours");
    refused(&mut s, "colorPalette.addColor", json!({}), "palette 1 has 1 colours");
}

#[test]
fn generate_dark_palette_appends_a_dark_palette_that_cpal_version_1_flags() {
    let mut s = palette_font();
    // A glyph whose palette layer draws something, so that COLR and CPAL are written.
    s.edit("dot", |font| {
        let master = font.masters[0].id.clone();
        let mut dot = Glyph::new("dot");
        let mut base = Layer::new(master.as_str(), 500.0);
        base.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 400.0, 400.0)));
        dot.layers.push(base);
        let mut colour = palette_layer(&master, "d0", PaletteEntry::Index(1), None);
        colour.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 400.0, 400.0)));
        dot.layers.push(colour);
        font.glyphs.push(dot);
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "color.generateDarkPalette", json!({"palette": 0}));
    assert_eq!((r["palette"].clone(), r["palettes"].clone(), r["rows"].clone(), r["name"].clone()), (json!(2), json!(3), json!(3), json!("Dark")));
    let listed = palettes(&mut s, json!({}));
    assert_eq!(listed["palettes"][2], r["colors"]);
    assert_eq!(listed["palettes"][0], json!(["#FF0000FF", "#00FF00FF", "#0000FFFF"]), "the source is unchanged");
    // Pure red keeps its hue: red is still the strongest channel.
    let red = fontcraft_color::Rgba::parse_hex(r["colors"][0].as_str().unwrap()).unwrap();
    assert!(red.r > red.g && red.r > red.b && red.a == 255, "{red:?}");

    let font = s.font().unwrap();
    let stored = font.info.params.get("Color Palettes").unwrap();
    let info = fontcraft_color::palette::palette_info(stored);
    assert_eq!(info.len(), 3);
    assert!(info[2].dark && !info[2].light && info[2].name.as_deref() == Some("Dark"));
    assert!(!info[0].dark && !info[1].dark);

    let out = fontcraft_compile::compile(
        font,
        &font.default_target().unwrap(),
        fontcraft_compile::Flavour::TrueType,
        &fontcraft_compile::Options::default(),
    )
    .unwrap();
    use skrifa::raw::TableProvider;
    let cpal = skrifa::FontRef::new(&out.bytes).unwrap().cpal().unwrap();
    assert_eq!(cpal.version(), 1);
    let types: Vec<u32> = cpal.palette_types_array().unwrap().unwrap().iter().map(|t| t.get().bits()).collect();
    assert_eq!(types, [0, 0, 2]);
    // The export report says the same before exporting.
    let described = run(&mut s, "color.describeExport", json!({}));
    let cpal = &described["exports"][0]["cpal"];
    assert_eq!(
        (cpal["version"].clone(), cpal["types"].clone(), cpal["labels"].clone()),
        (json!(1), json!([0, 0, 2]), json!([null, null, "Dark"])),
        "{described}"
    );

    // The palette commands keep the dictionary form and its flag.
    run(&mut s, "colorPalette.setColor", json!({"palette": 2, "index": 0, "color": "#FFEEDD"}));
    let stored = s.font().unwrap().info.params.get("Color Palettes").unwrap();
    assert!(fontcraft_color::palette::palette_info(stored)[2].dark);

    refused(&mut s, "color.generateDarkPalette", json!({"palette": 9}), "no palette 9");
    let mut empty = Session::with_document();
    refused(&mut empty, "color.generateDarkPalette", json!({}), "no colour palettes");
}
