//! A UFO's font-wide colour layer list (`com.github.googlei18n.ufo2ft.colorLayers`) with its
//! palettes opens as palette layers and exports the `COLR` and `CPAL` tables the lib describes,
//! the layer records pointing at the UFO's own layer glyphs.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_compile::{Flavour, Options, compile};
use fontcraft_io_ufo::Files;
use fontcraft_model::{PaletteEntry, Shape};
use skrifa::FontRef;
use skrifa::raw::TableProvider;

fn plist(body: &str) -> Vec<u8> {
    format!(
        "<?xml version='1.0' encoding='UTF-8'?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n{body}\n</plist>\n"
    )
    .into_bytes()
}

fn glif(name: &str, x: i32) -> Vec<u8> {
    let r = x + 200;
    format!(
        "<?xml version='1.0' encoding='UTF-8'?>\n<glyph name=\"{name}\" format=\"2\">\n  <advance width=\"600\"/>\n  <outline>\n    <contour>\n      <point x=\"{x}\" y=\"0\" type=\"line\"/>\n      <point x=\"{r}\" y=\"0\" type=\"line\"/>\n      <point x=\"{r}\" y=\"700\" type=\"line\"/>\n      <point x=\"{x}\" y=\"700\" type=\"line\"/>\n    </contour>\n  </outline>\n</glyph>\n"
    )
    .into_bytes()
}

/// `a` in two colours drawn by `a.color0` and `a.color1`, `b` in one by `b.color0`, two
/// palettes of two colours.
fn ufo() -> Files {
    ufo_with([[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [0.0, 1.0, 0.0, 1.0], [1.0, 1.0, 0.0, 1.0]])
}

/// [`ufo`] with the palettes' colours given: the first two make palette 0, the others palette 1.
fn ufo_with(colours: [[f64; 4]; 4]) -> Files {
    let names = ["a", "a.color0", "a.color1", "b", "b.color0"];
    let mut files = Files::new();
    files.insert(
        "metainfo.plist".into(),
        plist("<dict>\n<key>creator</key>\n<string>org.example</string>\n<key>formatVersion</key>\n<integer>3</integer>\n</dict>"),
    );
    files.insert(
        "fontinfo.plist".into(),
        plist("<dict>\n<key>familyName</key>\n<string>Listed</string>\n<key>unitsPerEm</key>\n<integer>1000</integer>\n</dict>"),
    );
    files.insert(
        "layercontents.plist".into(),
        plist("<array>\n<array>\n<string>public.default</string>\n<string>glyphs</string>\n</array>\n</array>"),
    );
    let contents: String = names.iter().map(|n| format!("<key>{n}</key>\n<string>{n}.glif</string>\n")).collect();
    files.insert("glyphs/contents.plist".into(), plist(&format!("<dict>\n{contents}</dict>")));
    for (x, n) in (0..).step_by(50).zip(names) {
        files.insert(format!("glyphs/{n}.glif"), glif(n, x));
    }
    let pair = |n: &str, i: u16| format!("<array>\n<string>{n}</string>\n<integer>{i}</integer>\n</array>\n");
    let colour = |c: [f64; 4]| format!("<array>\n{}</array>\n", c.iter().map(|v| format!("<real>{v:?}</real>\n")).collect::<String>());
    let order: String = names.iter().map(|n| format!("<string>{n}</string>\n")).collect();
    files.insert(
        "lib.plist".into(),
        plist(&format!(
            "<dict>\n<key>com.github.googlei18n.ufo2ft.colorLayers</key>\n<dict>\n<key>a</key>\n<array>\n{}{}</array>\n<key>b</key>\n<array>\n{}</array>\n</dict>\n\
             <key>com.github.googlei18n.ufo2ft.colorPalettes</key>\n<array>\n<array>\n{}{}</array>\n<array>\n{}{}</array>\n</array>\n\
             <key>public.glyphOrder</key>\n<array>\n{order}</array>\n</dict>",
            pair("a.color0", 0),
            pair("a.color1", 1),
            pair("b.color0", 0),
            colour(colours[0]),
            colour(colours[1]),
            colour(colours[2]),
            colour(colours[3]),
        )),
    );
    files
}

#[test]
fn a_ufo_colour_layer_list_exports_colr_and_cpal_as_the_lib_says() {
    let font = fontcraft_io_ufo::ufo::read(&ufo()).unwrap();
    let a = font.glyph("a").unwrap();
    let layers: Vec<(String, Option<PaletteEntry>)> = a
        .layers
        .iter()
        .filter(|l| !l.is_master_layer())
        .map(|l| match l.shapes.as_slice() {
            [Shape::Component(c)] => (c.glyph.clone(), l.info.attrs.color_palette),
            other => (format!("{other:?}"), None),
        })
        .collect();
    assert_eq!(layers, [("a.color0".to_string(), Some(PaletteEntry::Index(0))), ("a.color1".to_string(), Some(PaletteEntry::Index(1)))]);

    let out = compile(&font, &font.default_target().unwrap(), Flavour::TrueType, &Options::default()).unwrap();
    let order: Vec<&str> = out.glyph_order.iter().map(String::as_str).filter(|n| *n != ".notdef").collect();
    assert_eq!(order, ["a", "a.color0", "a.color1", "b", "b.color0"], "the layer glyphs are the UFO's own: {:?}", out.warnings);
    let f = FontRef::new(&out.bytes).unwrap();
    let colr = f.colr().unwrap();
    assert_eq!(colr.version(), 0);
    let records = colr.layer_records().unwrap().unwrap();
    let name = |g: skrifa::raw::types::GlyphId16| out.glyph_order[usize::from(g.to_u16())].as_str();
    let got: Vec<(&str, Vec<(&str, u16)>)> = colr
        .base_glyph_records()
        .unwrap()
        .unwrap()
        .iter()
        .map(|b| {
            let first = usize::from(b.first_layer_index());
            let own = &records[first..first + usize::from(b.num_layers())];
            (name(b.glyph_id()), own.iter().map(|l| (name(l.glyph_id()), l.palette_index())).collect())
        })
        .collect();
    assert_eq!(got, [("a", vec![("a.color0", 0), ("a.color1", 1)]), ("b", vec![("b.color0", 0)])]);

    assert_eq!(cpal_palettes(&f), [vec![(255, 0, 0, 255), (0, 0, 255, 255)], vec![(0, 255, 0, 255), (255, 255, 0, 255)]]);
}

/// The colours of each palette of the font's `CPAL` table.
fn cpal_palettes(f: &FontRef) -> Vec<Vec<(u8, u8, u8, u8)>> {
    let cpal = f.cpal().unwrap();
    let colours = cpal.color_records_array().unwrap().unwrap();
    let entries = usize::from(cpal.num_palette_entries());
    cpal.color_record_indices()
        .iter()
        .map(|i| colours[usize::from(i.get())..usize::from(i.get()) + entries].iter().map(|c| (c.red, c.green, c.blue, c.alpha)).collect())
        .collect()
}

#[test]
fn palette_colours_with_four_decimals_open_as_color_palettes_and_export_cpal() {
    // Four-place components, as a decompiled font's version 4 palettes are written (128 → 0.502):
    // the lib key is typed as the font's Color Palettes rather than kept as user data.
    let font =
        fontcraft_io_ufo::ufo::read(&ufo_with([[0.502, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.502], [0.0, 0.2, 0.0, 1.0], [1.0, 1.0, 0.0, 1.0]])).unwrap();
    assert!(font.info.params.get("Color Palettes").is_some(), "{:?}", font.masters[0].user_data);
    assert!(font.masters[0].user_data.get("com.github.googlei18n.ufo2ft.colorPalettes").is_none());
    let out = compile(&font, &font.default_target().unwrap(), Flavour::TrueType, &Options::default()).unwrap();
    let f = FontRef::new(&out.bytes).unwrap();
    assert_eq!(cpal_palettes(&f), [vec![(128, 0, 0, 255), (0, 0, 255, 128)], vec![(0, 51, 0, 255), (255, 255, 0, 255)]]);
}

#[test]
fn a_ufo_colour_layer_paint_exports_colr_version_1_with_the_ufo_s_layer_glyphs() {
    let mut files = ufo();
    let pair = |n: &str, i: u16| format!("<array>\n<string>{n}</string>\n<integer>{i}</integer>\n</array>\n");
    let pairs = format!("<key>a</key>\n<array>\n{}{}</array>\n", pair("a.color0", 0), pair("a.color1", 1));
    let int = |k: &str, v: i64| format!("<key>{k}</key>\n<integer>{v}</integer>\n");
    let stop = |i: i64, at: i64| format!("<dict>\n{}{}{}</dict>\n", int("Alpha", 1), int("PaletteIndex", i), int("StopOffset", at));
    let solid = format!("<dict>\n<key>Alpha</key>\n<real>0.5</real>\n{}{}</dict>\n", int("Format", 2), int("PaletteIndex", 0));
    // `a.color1` runs from 100 to 300 across: a gradient from its left to its right edge.
    let line = format!(
        "<key>ColorLine</key>\n<dict>\n<key>ColorStop</key>\n<array>\n{}{}</array>\n<key>Extend</key>\n<string>pad</string>\n</dict>\n",
        stop(0, 0),
        stop(1, 1)
    );
    let points: String = [("x0", 100), ("x1", 300), ("x2", 100), ("y0", 0), ("y1", 0), ("y2", -200)].iter().map(|(k, v)| int(k, *v)).collect();
    let linear = format!("<dict>\n{line}{}{points}</dict>\n", int("Format", 4));
    let glyph =
        |n: &str, paint: &str| format!("<dict>\n{}<key>Glyph</key>\n<string>{n}</string>\n<key>Paint</key>\n{paint}</dict>\n", int("Format", 10));
    let paint = format!(
        "<key>a</key>\n<dict>\n{}<key>Layers</key>\n<array>\n{}{}</array>\n</dict>\n",
        int("Format", 1),
        glyph("a.color0", &solid),
        glyph("a.color1", &linear)
    );
    let lib = String::from_utf8(files["lib.plist"].clone()).unwrap();
    assert!(lib.contains(&pairs));
    files.insert("lib.plist".into(), lib.replace(&pairs, &paint).into_bytes());

    let font = fontcraft_io_ufo::ufo::read(&files).unwrap();
    let a = font.glyph("a").unwrap();
    let full: Vec<&str> = a
        .layers
        .iter()
        .filter(|l| l.info.attrs.full_color)
        .filter_map(|l| match l.shapes.as_slice() {
            [Shape::Component(c)] => Some(c.glyph.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(full, ["a.color0", "a.color1"], "full-colour layers of the UFO's layer glyphs");

    let out = compile(&font, &font.default_target().unwrap(), Flavour::TrueType, &Options::default()).unwrap();
    let order: Vec<&str> = out.glyph_order.iter().map(String::as_str).filter(|n| *n != ".notdef").collect();
    assert_eq!(order, ["a", "a.color0", "a.color1", "b", "b.color0"], "the layer glyphs are the UFO's own: {:?}", out.warnings);
    let f = FontRef::new(&out.bytes).unwrap();
    assert_eq!(f.colr().unwrap().version(), 1, "a gradient and an alpha need version 1");
}
