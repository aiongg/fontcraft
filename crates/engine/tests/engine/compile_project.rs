//! The project compile of `font.compile` (M12.40). Every expected value was read from fonts
//! the reference compiler (AFDKO makeotf 5.0.1) built from the same UFO and files.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use serde_json::{Value, json};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, MetadataProvider};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-project-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn plist(body: &str) -> String {
    format!(
        "<?xml version='1.0' encoding='UTF-8'?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n{body}\n</plist>\n"
    )
}

/// The glyphs of the synthetic UFO: name, code point in the source, advance.
const GLYPHS: [(&str, Option<u32>, u32); 11] = [
    (".notdef", None, 500),
    ("space", Some(0x20), 250),
    ("zdev", None, 520),
    ("B", Some(0x42), 600),
    ("A", Some(0x41), 600),
    ("a.dev", None, 500),
    ("uni00E9", None, 500),
    ("hyphen", Some(0x2D), 300),
    ("mu", None, 500),
    ("smile", None, 600),
    ("extra", Some(0x2603), 400),
];

const GOADB: &str = ".notdef .notdef\nspace space\nB B\nA A\na a.dev\nz zdev\neacute uni00E9\nuni1F600 smile u1F600\nhyphen hyphen\nmu mu\n";

/// Writes `font.ufo` with [`GLYPHS`] and a kerning feature into `dir`.
fn write_ufo(dir: &Path) -> PathBuf {
    let ufo = dir.join("font.ufo");
    let glyphs = ufo.join("glyphs");
    std::fs::create_dir_all(&glyphs).unwrap();
    std::fs::write(
        ufo.join("metainfo.plist"),
        plist("<dict><key>creator</key><string>org.example</string><key>formatVersion</key><integer>3</integer></dict>"),
    )
    .unwrap();
    std::fs::write(
        ufo.join("fontinfo.plist"),
        plist(
            "<dict><key>familyName</key><string>Synth</string><key>styleName</key><string>Regular</string>\
             <key>postscriptFontName</key><string>Synth-Regular</string><key>unitsPerEm</key><integer>1000</integer>\
             <key>ascender</key><integer>750</integer><key>descender</key><integer>-250</integer><key>capHeight</key><integer>700</integer>\
             <key>xHeight</key><integer>500</integer><key>versionMajor</key><integer>1</integer><key>versionMinor</key><integer>0</integer></dict>",
        ),
    )
    .unwrap();
    std::fs::write(ufo.join("layercontents.plist"), plist("<array><array><string>public.default</string><string>glyphs</string></array></array>"))
        .unwrap();
    let mut contents = String::new();
    let mut order = String::new();
    for (i, (name, code, width)) in GLYPHS.iter().enumerate() {
        let file = format!("g{i}.glif");
        contents.push_str(&format!("<key>{name}</key><string>{file}</string>"));
        order.push_str(&format!("<string>{name}</string>"));
        let unicode = code.map(|c| format!("<unicode hex=\"{c:04X}\"/>")).unwrap_or_default();
        let outline = if *name == "space" {
            String::new()
        } else {
            let r = width - 50;
            format!(
                "<outline><contour><point x=\"50\" y=\"0\" type=\"line\"/><point x=\"50\" y=\"700\" type=\"line\"/><point x=\"{r}\" y=\"700\" type=\"line\"/><point x=\"{r}\" y=\"0\" type=\"line\"/></contour></outline>"
            )
        };
        std::fs::write(
            glyphs.join(&file),
            format!(
                "<?xml version='1.0' encoding='UTF-8'?>\n<glyph name=\"{name}\" format=\"2\"><advance width=\"{width}\"/>{unicode}{outline}</glyph>\n"
            ),
        )
        .unwrap();
    }
    std::fs::write(glyphs.join("contents.plist"), plist(&format!("<dict>{contents}</dict>"))).unwrap();
    std::fs::write(ufo.join("lib.plist"), plist(&format!("<dict><key>public.glyphOrder</key><array>{order}</array></dict>"))).unwrap();
    std::fs::write(ufo.join("features.fea"), "languagesystem DFLT dflt;\nfeature kern { pos A B -40; } kern;\n").unwrap();
    ufo
}

/// Opens the UFO in `dir`, runs `font.compile` into `out.otf` with `extra`, and returns the bytes
/// and the result.
fn build(dir: &Path, extra: Value) -> (Vec<u8>, Value) {
    let ufo = write_ufo(dir);
    let mut s = Session::new();
    s.execute("file.open", &json!({ "path": ufo })).unwrap();
    let out = dir.join("out.otf");
    let mut params = json!({"formats": ["otf"], "destination": out, "timestamp": 0});
    for (k, v) in extra.as_object().unwrap() {
        params[k] = v.clone();
    }
    let result = s.execute("font.compile", &params).unwrap_or_else(|e| panic!("{params}: {e}"));
    (std::fs::read(&out).unwrap(), result)
}

fn order(bytes: &[u8]) -> Vec<String> {
    FontRef::new(bytes).unwrap().glyph_names().iter().map(|(_, n)| n.to_string()).collect()
}

/// Code point → glyph name.
fn cmap(bytes: &[u8]) -> BTreeMap<u32, String> {
    let font = FontRef::new(bytes).unwrap();
    let names = font.glyph_names();
    font.charmap().mappings().map(|(c, g)| (c, names.get(g).unwrap().to_string())).collect()
}

/// (name ID, platform, language) → text.
fn names(bytes: &[u8]) -> BTreeMap<(u16, u16, u16), String> {
    let font = FontRef::new(bytes).unwrap();
    let name = font.name().unwrap();
    name.name_record()
        .iter()
        .map(|r| ((r.name_id().to_u16(), r.platform_id(), r.language_id()), r.string(name.string_data()).unwrap().to_string()))
        .collect()
}

fn win(n: &BTreeMap<(u16, u16, u16), String>, id: u16) -> Option<&str> {
    n.get(&(id, 3, 0x409)).map(String::as_str)
}

#[test]
fn a_development_project_compile_orders_maps_and_names_as_the_reference() {
    let dir = temp("dev");
    let (bytes, result) = build(&dir, json!({"projectOptions": []}));
    assert_eq!(order(&bytes), [".notdef", "space", "hyphen", "A", "B", "mu", "zdev", "a.dev", "uni00E9", "smile", "extra"]);
    let expected: BTreeMap<u32, String> = [(0x20, "space"), (0x2D, "hyphen"), (0x41, "A"), (0x42, "B"), (0xB5, "mu"), (0xE9, "uni00E9")]
        .into_iter()
        .map(|(c, n)| (c, n.to_string()))
        .collect();
    assert_eq!(cmap(&bytes), expected, "the source's U+2603 on `extra` is not used");
    let n = names(&bytes);
    assert_eq!((win(&n, 1), win(&n, 2), win(&n, 4), win(&n, 6)), (Some("Synth"), Some("Regular"), Some("Synth"), Some("Synth-Regular")));
    assert!(win(&n, 16).is_none() && win(&n, 17).is_none(), "{n:?}");
    assert_eq!(n.get(&(1, 1, 0)).map(String::as_str), Some("Synth"), "Macintosh names are written");
    let font = FontRef::new(&bytes).unwrap();
    assert!(font.table_data(skrifa::raw::types::Tag::new(b"DSIG")).is_none());
    let warnings = result.to_string();
    assert!(warnings.contains("extra"), "the replaced code point is reported: {warnings}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_release_project_compile_renames_orders_and_signs_by_the_goadb() {
    let dir = temp("release");
    std::fs::write(dir.join("GlyphOrderAndAliasDB"), GOADB).unwrap();
    let (bytes, _) = build(&dir, json!({"projectOptions": ["release"], "projectDefaults": true}));
    assert_eq!(order(&bytes), [".notdef", "space", "B", "A", "a", "z", "eacute", "uni1F600", "hyphen", "mu", "extra"]);
    let map = cmap(&bytes);
    for (c, n) in [(0x61, "a"), (0x7A, "z"), (0xE9, "eacute"), (0x1F600, "uni1F600"), (0xB5, "mu"), (0x41, "A")] {
        assert_eq!(map.get(&c).map(String::as_str), Some(n), "U+{c:04X}");
    }
    assert!(!map.values().any(|n| n == "extra"));
    let font = FontRef::new(&bytes).unwrap();
    assert_eq!(font.table_data(skrifa::raw::types::Tag::new(b"DSIG")).unwrap().as_bytes(), [0, 0, 0, 1, 0, 0, 0, 1]);
    assert!(font.gpos().is_ok(), "the kerning written with source names is kept");

    // Subsetting leaves out the glyph the GOADB does not list.
    let dir2 = temp("subset");
    std::fs::write(dir2.join("GlyphOrderAndAliasDB"), GOADB).unwrap();
    let goadb = dir2.join("GlyphOrderAndAliasDB");
    let (bytes, _) = build(&dir2, json!({"projectOptions": ["apply-goadb", "subset-to-goadb"], "goadb": goadb}));
    assert_eq!(order(&bytes), [".notdef", "space", "B", "A", "a", "z", "eacute", "uni1F600", "hyphen", "mu"]);
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&dir2).unwrap();
}

#[test]
fn the_goadb_alone_gives_code_points_by_source_name() {
    let dir = temp("unicodes");
    let goadb = dir.join("GOADB");
    std::fs::write(&goadb, GOADB).unwrap();
    let (bytes, _) = build(&dir, json!({"goadb": goadb}));
    assert_eq!(order(&bytes)[2], "hyphen", "no renaming or reordering without apply-goadb");
    let map = cmap(&bytes);
    assert_eq!(map.get(&0x1F600).map(String::as_str), Some("smile"));
    assert_eq!(map.get(&0x41).map(String::as_str), Some("A"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn menu_names_come_from_the_font_menu_name_db_with_the_style_link() {
    let dir = temp("menu");
    let db = dir.join("FontMenuNameDB");
    std::fs::write(&db, "[Synth-Regular]\n\tf=Synth Pro\n\ts=Light Italic\n\tl=Synth Pro Light\n").unwrap();
    let (bytes, _) = build(&dir, json!({"menuNames": db, "projectOptions": ["italic"]}));
    let n = names(&bytes);
    assert_eq!(win(&n, 1), Some("Synth Pro Light"));
    assert_eq!(win(&n, 2), Some("Italic"));
    assert_eq!(win(&n, 4), Some("Synth Pro Light Italic"));
    assert_eq!(win(&n, 16), Some("Synth Pro"));
    assert_eq!(win(&n, 17), Some("Light Italic"));
    let os2 = FontRef::new(&bytes).unwrap().os2().unwrap();
    assert_eq!(os2.fs_selection().bits() & 0x61, 0x01, "italic, not bold, not regular");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_legacy_cmap_options_double_map_and_stub() {
    let dir = temp("dbl");
    let (bytes, _) = build(&dir, json!({"projectOptions": ["legacy-double-mapping"]}));
    let map = cmap(&bytes);
    for (c, n) in [(0xA0, "space"), (0xAD, "hyphen"), (0x3BC, "mu"), (0xB5, "mu")] {
        assert_eq!(map.get(&c).map(String::as_str), Some(n), "U+{c:04X}");
    }
    let dir2 = temp("stub");
    let (bytes, _) = build(&dir2, json!({"projectOptions": ["stub-cmap4"]}));
    let map = cmap(&bytes);
    assert_eq!(map.into_iter().collect::<Vec<_>>(), [(0x20, "space".to_string())]);
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&dir2).unwrap();
}

#[test]
fn a_refused_project_compile_writes_nothing() {
    let dir = temp("refused");
    let ufo = write_ufo(&dir);
    let mut s = Session::new();
    s.execute("file.open", &json!({ "path": ufo })).unwrap();
    let out = dir.join("out.otf");
    let bad_db = dir.join("FontMenuNameDB");
    std::fs::write(&bad_db, "f=No entry line\n").unwrap();
    for extra in [json!({"projectOptions": ["synthetic-glyphs"]}), json!({"projectOptions": ["no-such-option"]}), json!({"menuNames": bad_db})] {
        let mut params = json!({"formats": ["otf"], "destination": out});
        for (k, v) in extra.as_object().unwrap() {
            params[k] = v.clone();
        }
        assert!(s.execute("font.compile", &params).is_err(), "{params}");
        assert!(!out.exists(), "{params}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
