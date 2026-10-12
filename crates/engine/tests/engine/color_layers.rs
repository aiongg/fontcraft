//! Colour layer commands (M11.3): converting a layer to each colour kind, palette index, iColor
//! size and image, svg image, the list query, and colour layers following the master's width.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fontcraft_engine::Session;
use fontcraft_engine::model::{Layer, Node, NodeKind, PaletteEntry, Path, Shape, Transform};
use fontcraft_testkit::snapshot;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn fails(s: &mut Session, id: &str, p: Value, mentions: &str) {
    let before = snapshot(s);
    let e = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} {p} should fail"));
    assert!(e.to_string().contains(mentions), "{id} {p}: `{e}` should mention `{mentions}`");
    assert_eq!(snapshot(s), before, "{id} {p}: a refused command changes nothing");
}

/// The active font, for comparing before and after an undo.
fn font(s: &Session) -> Value {
    serde_json::to_value(s.font().unwrap()).unwrap()
}

/// The path an unsaved document stores for `relative`: absolute, taken from the working directory.
fn absolute(relative: &str) -> String {
    std::path::absolute(relative).unwrap().to_str().unwrap().to_string()
}

fn layer(s: &Session, glyph: &str, id: &str) -> Layer {
    s.font().unwrap().glyph(glyph).unwrap().layers.iter().find(|l| l.id == id).unwrap().clone()
}

/// `A` and `E` (600 wide in master `m01`) each get a plain extra layer `x1` and `x2`, 480 wide.
/// `E`'s master layer holds two one-node paths at (50, -100) and (550, 700), the corners of the
/// box an iColor image is placed in.
fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        for name in ["A", "E"] {
            let mut g = font.glyph_mut(name)?;
            for id in ["x1", "x2"] {
                let mut l = Layer::new("m01", 480.0);
                l.id = id.to_string();
                g.layers.push(l);
            }
        }
        let mut e = font.glyph_mut("E")?;
        let master = e.layer_mut("m01").unwrap();
        master.shapes.push(Shape::Path(Path::new(false, vec![Node::new(50.0, -100.0, NodeKind::Line)])));
        master.shapes.push(Shape::Path(Path::new(false, vec![Node::new(550.0, 700.0, NodeKind::Line)])));
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn layer_convert_commands_set_one_kind_take_the_master_width_and_refuse_master_layers() {
    let mut s = session();
    let r = run(&mut s, "layer.convertToColorPalette", json!({"glyph": "A", "layer": "x1"}));
    assert_eq!((r["kind"].as_str(), r["label"].as_str(), r["index"].as_u64()), (Some("palette"), Some("Color 0"), Some(0)));
    let l = layer(&s, "A", "x1");
    assert_eq!(l.info.attrs.color_palette, Some(PaletteEntry::Index(0)));
    assert_eq!(l.width, 600.0, "a colour layer takes its master layer's width");

    // Each conversion replaces the kind before it: one colour attribute at a time.
    let r = run(&mut s, "layer.convertToColor", json!({"glyph": "A", "layer": "x1"}));
    assert_eq!((r["kind"].as_str(), r["label"].as_str()), (Some("fullColor"), Some("Color")));
    let a = layer(&s, "A", "x1").info.attrs.clone();
    assert!(a.full_color && a.color_palette.is_none() && a.sbix_size.is_none() && !a.svg);

    let r = run(&mut s, "layer.convertToIColor", json!({"glyph": "A", "layer": "x1"}));
    assert_eq!((r["label"].as_str(), r["size"].as_u64()), (Some("iColor 128"), Some(128)), "a new iColor layer is 128 pixels per em");
    let a = layer(&s, "A", "x1").info.attrs.clone();
    assert!(!a.full_color && a.sbix_size == Some(128));

    let r = run(&mut s, "layer.convertToSvg", json!({"glyph": "A", "layer": "x1"}));
    assert_eq!((r["kind"].as_str(), r["label"].as_str()), (Some("svg"), Some("svg")));
    let a = layer(&s, "A", "x1").info.attrs.clone();
    assert!(a.svg && a.sbix_size.is_none());

    let r = run(&mut s, "layer.convertToColorPalette", json!({"glyph": "A", "layer": "x1", "index": "*"}));
    assert_eq!(r["label"].as_str(), Some("Color *"));
    let a = layer(&s, "A", "x1").info.attrs.clone();
    assert!(!a.svg && a.color_palette == Some(PaletteEntry::Foreground));

    // Converting to the same kind without a parameter keeps the layer as it is.
    run(&mut s, "layer.convertToIColor", json!({"glyph": "A", "layer": "x2", "size": 64}));
    let r = run(&mut s, "layer.convertToIColor", json!({"glyph": "A", "layer": "x2"}));
    assert_eq!(r["size"].as_u64(), Some(64));

    for id in ["layer.convertToColorPalette", "layer.convertToColor", "layer.convertToIColor", "layer.convertToSvg"] {
        fails(&mut s, id, json!({"glyph": "A", "layer": "m01"}), "master layer");
        fails(&mut s, id, json!({"glyph": "A", "layer": "nope"}), "nope");
        fails(&mut s, id, json!({"glyph": "A"}), "layer");
    }
    fails(&mut s, "layer.convertToIColor", json!({"glyph": "A", "layer": "x1", "size": 0}), "size");
    fails(&mut s, "layer.convertToIColor", json!({"glyph": "A", "layer": "x1", "size": 4097}), "size");

    // Undo takes back one conversion at a time.
    let before = font(&s);
    run(&mut s, "layer.convertToColor", json!({"glyph": "E", "layer": "x1"}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(font(&s), before);
}

#[test]
fn layer_set_color_palette_index_takes_a_row_or_the_text_colour_on_palette_layers_only() {
    let mut s = session();
    run(&mut s, "layer.convertToColorPalette", json!({"glyph": "A", "layer": "x1", "index": 1}));
    let r = run(&mut s, "layer.setColorPaletteIndex", json!({"glyph": "A", "layer": "x1", "index": 3}));
    assert_eq!((r["label"].as_str(), r["index"].as_u64()), (Some("Color 3"), Some(3)));
    run(&mut s, "layer.setColorPaletteIndex", json!({"glyph": "A", "layer": "x1", "index": "*"}));
    assert_eq!(layer(&s, "A", "x1").info.attrs.color_palette, Some(PaletteEntry::Foreground));

    fails(&mut s, "layer.setColorPaletteIndex", json!({"glyph": "A", "layer": "x1", "index": -1}), "index");
    fails(&mut s, "layer.setColorPaletteIndex", json!({"glyph": "A", "layer": "x1", "index": 65534}), "index");
    fails(&mut s, "layer.setColorPaletteIndex", json!({"glyph": "A", "layer": "x1", "index": "red"}), "index");
    fails(&mut s, "layer.setColorPaletteIndex", json!({"glyph": "A", "layer": "x1"}), "index");
    fails(&mut s, "layer.setColorPaletteIndex", json!({"glyph": "A", "layer": "x2", "index": 0}), "Color Palette layer");
}

#[test]
fn layer_set_icolor_image_places_the_bitmap_at_the_master_bounds_and_layer_set_icolor_size_rescales_it() {
    let mut s = session();
    run(&mut s, "layer.convertToIColor", json!({"glyph": "E", "layer": "x1", "size": 64}));
    let r = run(&mut s, "layer.setIColorImage", json!({"glyph": "E", "layer": "x1", "path": "Images/E 64.png"}));
    // The document is unsaved: a relative path is stored absolute, as Add Image stores it.
    let stored = absolute("Images/E 64.png");
    assert_eq!(r["image"].as_str(), Some(stored.as_str()));
    let image = layer(&s, "E", "x1").info.image.clone().unwrap();
    assert_eq!(image.path.as_deref(), Some(stored.as_str()));
    // Bottom left of the master's path bounds, 1000 / 64 units per pixel.
    assert_eq!(image.transform, Transform::Decomposed { pos: (50.0, -100.0), scale: (15.625, 15.625), angle: 0.0, slant: (0.0, 0.0) });

    let r = run(&mut s, "layer.setIColorSize", json!({"glyph": "E", "layer": "x1", "size": 128}));
    assert_eq!(r["label"].as_str(), Some("iColor 128"));
    let image = layer(&s, "E", "x1").info.image.clone().unwrap();
    assert_eq!(
        image.transform,
        Transform::Decomposed { pos: (50.0, -100.0), scale: (7.8125, 7.8125), angle: 0.0, slant: (0.0, 0.0) },
        "twice the pixels per em draws the image half as large"
    );

    // A master layer without paths places the image at the origin.
    run(&mut s, "layer.convertToIColor", json!({"glyph": "A", "layer": "x1", "size": 100}));
    run(&mut s, "layer.setIColorImage", json!({"glyph": "A", "layer": "x1", "path": "a.JPG"}));
    let image = layer(&s, "A", "x1").info.image.clone().unwrap();
    assert_eq!(image.transform, Transform::Decomposed { pos: (0.0, 0.0), scale: (10.0, 10.0), angle: 0.0, slant: (0.0, 0.0) });

    fails(&mut s, "layer.setIColorImage", json!({"glyph": "E", "layer": "x1", "path": "E.svg"}), "path");
    fails(&mut s, "layer.setIColorImage", json!({"glyph": "E", "layer": "x1", "path": ""}), "path");
    fails(&mut s, "layer.setIColorImage", json!({"glyph": "E", "layer": "x1", "path": "a\u{7}.png"}), "path");
    fails(&mut s, "layer.setIColorImage", json!({"glyph": "E", "layer": "x2", "path": "E.png"}), "iColor layer");
    fails(&mut s, "layer.setIColorSize", json!({"glyph": "E", "layer": "x2", "size": 32}), "iColor layer");
    fails(&mut s, "layer.setIColorSize", json!({"glyph": "E", "layer": "x1", "size": 5000}), "size");
}

#[test]
fn layer_set_icolor_size_refuses_to_scale_the_image_beyond_the_coordinate_limit() {
    let mut s = session();
    run(&mut s, "layer.convertToIColor", json!({"glyph": "E", "layer": "x1", "size": 4096}));
    run(&mut s, "layer.setIColorImage", json!({"glyph": "E", "layer": "x1", "path": "E.png"}));
    s.edit("huge", |font| {
        let mut g = font.glyph_mut("E")?;
        let l = g.layers.iter_mut().find(|l| l.id == "x1").unwrap();
        l.info.image.as_mut().unwrap().transform =
            Transform::Decomposed { pos: (0.0, 0.0), scale: (500_000.0, 500_000.0), angle: 0.0, slant: (0.0, 0.0) };
        Ok(())
    })
    .unwrap();
    // From 4096 to 1 pixel per em multiplies the scale by 4096.
    fails(&mut s, "layer.setIColorSize", json!({"glyph": "E", "layer": "x1", "size": 1}), "beyond");
    // Twice the scale reaches the limit and is taken.
    run(&mut s, "layer.setIColorSize", json!({"glyph": "E", "layer": "x1", "size": 2048}));
    let image = layer(&s, "E", "x1").info.image.clone().unwrap();
    assert_eq!(image.transform, Transform::Decomposed { pos: (0.0, 0.0), scale: (1_000_000.0, 1_000_000.0), angle: 0.0, slant: (0.0, 0.0) });
}

#[test]
fn layer_set_svg_image_references_the_file_at_the_origin() {
    let mut s = session();
    run(&mut s, "layer.convertToSvg", json!({"glyph": "A", "layer": "x1"}));
    let r = run(&mut s, "layer.setSvgImage", json!({"glyph": "A", "layer": "x1", "path": "Images/F.svg"}));
    let stored = absolute("Images/F.svg");
    assert_eq!((r["kind"].as_str(), r["image"].as_str()), (Some("svg"), Some(stored.as_str())));
    let image = layer(&s, "A", "x1").info.image.clone().unwrap();
    assert_eq!(image.path.as_deref(), Some(stored.as_str()));
    assert!(image.transform.is_identity());
    fails(&mut s, "layer.setSvgImage", json!({"glyph": "A", "layer": "x1", "path": "F.png"}), "path");
    fails(&mut s, "layer.setSvgImage", json!({"glyph": "A", "layer": "x2", "path": "F.svg"}), "svg layer");
}

#[test]
fn layer_list_color_layers_reports_kinds_in_paint_order_with_their_files() {
    let mut s = session();
    run(&mut s, "layer.convertToColorPalette", json!({"glyph": "A", "layer": "x2", "index": 2}));
    run(&mut s, "layer.convertToSvg", json!({"glyph": "A", "layer": "x1"}));
    run(&mut s, "layer.setSvgImage", json!({"glyph": "A", "layer": "x1", "path": "missing/F.svg"}));
    run(&mut s, "layer.convertToIColor", json!({"glyph": "E", "layer": "x1", "size": 64}));

    let r = run(&mut s, "layer.listColorLayers", json!({}));
    let glyphs: Vec<&str> = r["glyphs"].as_array().unwrap().iter().map(|g| g["glyph"].as_str().unwrap()).collect();
    assert_eq!(glyphs, ["A", "E"], "every glyph with a colour layer, nothing else");
    let a = &r["glyphs"][0]["layers"];
    assert_eq!(a[0]["layer"], "x1", "file order: the first layer is painted at the bottom");
    assert_eq!(
        (a[0]["kind"].as_str(), a[0]["image"].as_str(), a[0]["exists"].as_bool()),
        (Some("svg"), Some(absolute("missing/F.svg").as_str()), Some(false))
    );
    assert_eq!((a[1]["kind"].as_str(), a[1]["index"].as_u64(), a[1]["label"].as_str()), (Some("palette"), Some(2), Some("Color 2")));
    assert_eq!(r["glyphs"][1]["layers"][0]["size"].as_u64(), Some(64));

    let r = run(&mut s, "layer.listColorLayers", json!({"glyphs": ["B"], "master": "m01"}));
    assert_eq!(r, json!({"glyphs": [{"glyph": "B", "layers": []}]}));
    assert!(s.execute("layer.listColorLayers", &json!({"master": "nope"})).is_err());
    assert!(s.execute("layer.listColorLayers", &json!({"glyphs": ["nope"]})).is_err());
}

#[test]
fn color_layers_follow_the_master_width_in_the_same_undo_step() {
    let mut s = session();
    run(&mut s, "layer.convertToIColor", json!({"glyph": "A", "layer": "x1"}));
    run(&mut s, "layer.convertToColorPalette", json!({"glyph": "A", "layer": "x2"}));
    run(&mut s, "layer.convertToSvg", json!({"glyph": "E", "layer": "x1"}));
    let before = font(&s);

    run(&mut s, "spacing.setWidth", json!({"glyphs": ["A", "E"], "value": 720}));
    assert_eq!((layer(&s, "A", "x1").width, layer(&s, "A", "x2").width), (720.0, 720.0));
    assert_eq!(layer(&s, "E", "x1").width, 720.0);
    assert_eq!(layer(&s, "E", "x2").width, 480.0, "a layer that is not a colour layer keeps its own width");

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(font(&s), before, "one undo step takes back the master and its colour layers");

    // A width set on a colour layer alone is not pulled back while the master stays.
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["A"], "layer": "x1", "value": 500}));
    assert_eq!(layer(&s, "A", "x1").width, 500.0);
}
