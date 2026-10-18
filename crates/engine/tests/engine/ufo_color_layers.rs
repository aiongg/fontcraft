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
            colour([1.0, 0.0, 0.0, 1.0]),
            colour([0.0, 0.0, 1.0, 1.0]),
            colour([0.0, 1.0, 0.0, 1.0]),
            colour([1.0, 1.0, 0.0, 1.0]),
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

    let cpal = f.cpal().unwrap();
    let colours = cpal.color_records_array().unwrap().unwrap();
    let entries = usize::from(cpal.num_palette_entries());
    let palettes: Vec<Vec<(u8, u8, u8, u8)>> = cpal
        .color_record_indices()
        .iter()
        .map(|i| colours[usize::from(i.get())..usize::from(i.get()) + entries].iter().map(|c| (c.red, c.green, c.blue, c.alpha)).collect())
        .collect();
    assert_eq!(palettes, [vec![(255, 0, 0, 255), (0, 0, 255, 255)], vec![(0, 255, 0, 255), (255, 255, 0, 255)]]);
}
