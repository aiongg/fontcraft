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

/// The glyph order `font.compilePlan` gives for the UFO in `dir` with `extra`, by written name.
fn planned_order(dir: &Path, extra: Value) -> Vec<String> {
    let ufo = write_ufo(dir);
    let mut s = Session::new();
    s.execute("file.open", &json!({ "path": ufo })).unwrap();
    let mut params = json!({"formats": ["otf"], "sections": ["glyphOrder", "productionNames"]});
    for (k, v) in extra.as_object().unwrap() {
        params[k] = v.clone();
    }
    let plan = s.execute("font.compilePlan", &params).unwrap_or_else(|e| panic!("{params}: {e}"));
    let written = |n: &str| plan["productionNames"].get(n).and_then(Value::as_str).unwrap_or(n).to_string();
    plan["glyphOrder"].as_array().unwrap().iter().map(|n| written(n.as_str().unwrap())).collect()
}

#[test]
fn the_compile_plan_gives_the_glyph_order_of_the_project_compile() {
    for (name, extra) in
        [("plan-dev", json!({"projectOptions": []})), ("plan-release", json!({"projectOptions": ["release"], "projectDefaults": true}))]
    {
        let dir = temp(name);
        std::fs::write(dir.join("GlyphOrderAndAliasDB"), GOADB).unwrap();
        let (bytes, _) = build(&dir, extra.clone());
        assert_eq!(planned_order(&dir, extra), order(&bytes), "{name}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// The glyphs of the metrics UFO as `.glif` outlines: name, file, advance, code point, outline.
const METRIC_GLYPHS: [(&str, &str, u32, Option<u32>, &str); 9] = [
    (
        ".notdef",
        "_notdef",
        500,
        None,
        r#"<contour><point x="50" y="0" type="line"/><point x="50" y="700" type="line"/><point x="450" y="700" type="line"/><point x="450" y="0" type="line"/></contour>"#,
    ),
    ("space", "space", 250, Some(0x20), ""),
    (
        "d",
        "d",
        560,
        Some(0x64),
        r#"<contour><point x="60" y="0" type="line"/><point x="60" y="760.4" type="line"/><point x="140.257" y="760.4" type="line"/><point x="140.257" y="0" type="line"/></contour>"#,
    ),
    (
        "O",
        "O_",
        700,
        Some(0x4F),
        r#"<contour><point x="350" y="-12.3" type="curve"/><point x="550" y="-12.3"/><point x="650" y="150"/><point x="650" y="350" type="curve"/><point x="650" y="550"/><point x="550" y="712"/><point x="350" y="712" type="curve"/><point x="150" y="712"/><point x="50" y="550"/><point x="50" y="350" type="curve"/><point x="50" y="150"/><point x="150" y="-12.3"/></contour>"#,
    ),
    (
        "o",
        "o",
        500,
        Some(0x6F),
        r#"<contour><point x="50" y="-10" type="line"/><point x="50" y="510" type="line"/><point x="450" y="510" type="line"/><point x="450" y="-10" type="line"/></contour>"#,
    ),
    (
        "p",
        "p",
        500,
        Some(0x70),
        r#"<contour><point x="50" y="-230.6" type="line"/><point x="50" y="500" type="line"/><point x="450" y="500" type="line"/><point x="450" y="-230.6" type="line"/></contour>"#,
    ),
    (
        "acutecomb",
        "acutecomb",
        0,
        None,
        r#"<contour><point x="-100" y="600" type="line"/><point x="-50" y="800" type="line"/><point x="0" y="800" type="line"/><point x="-60" y="600" type="line"/></contour>"#,
    ),
    ("Oacute", "O_acute", 720, Some(0xD3), r#"<component base="O"/><component base="acutecomb" xOffset="400" yOffset="150"/>"#),
    (
        "bar",
        "bar",
        300,
        None,
        r#"<contour><point x="100" y="0" type="line"/><point x="100" y="300" type="line"/><point x="200" y="300" type="line"/><point x="200" y="0" type="line"/></contour><contour><point x="150" y="100" type="line"/><point x="150" y="400" type="line"/><point x="250" y="400" type="line"/><point x="250" y="100" type="line"/></contour>"#,
    ),
];

/// Writes `Synth-Light.ufo` into `dir`: OpenType metric values, names and kerning the reference
/// does not read, glyphs whose boxes give its metrics, a composite whose advance is not its
/// base's, overlapping contours and fractional coordinates. `features` is its feature code.
fn write_metrics_ufo(dir: &Path, features: &str) -> PathBuf {
    let ufo = dir.join("Synth-Light.ufo");
    let glyphs = ufo.join("glyphs");
    std::fs::create_dir_all(&glyphs).unwrap();
    std::fs::write(
        ufo.join("metainfo.plist"),
        plist("<dict><key>creator</key><string>org.example</string><key>formatVersion</key><integer>3</integer></dict>"),
    )
    .unwrap();
    let info = [
        ("familyName", "<string>Synth</string>"),
        ("styleName", "<string>Light</string>"),
        ("postscriptFontName", "<string>Synth-Light</string>"),
        ("postscriptFullName", "<string>Synth Light</string>"),
        ("unitsPerEm", "<integer>1000</integer>"),
        ("ascender", "<integer>750</integer>"),
        ("descender", "<integer>-250</integer>"),
        ("capHeight", "<integer>700</integer>"),
        ("xHeight", "<integer>500</integer>"),
        ("versionMajor", "<integer>2</integer>"),
        ("versionMinor", "<integer>5</integer>"),
        ("copyright", "<string>Copyright Synth</string>"),
        ("trademark", "<string>Synth is a mark</string>"),
        ("openTypeNameDesigner", "<string>Someone</string>"),
        ("openTypeNameLicenseURL", "<string>https://example.com/l</string>"),
        ("openTypeOS2WeightClass", "<integer>200</integer>"),
        ("openTypeOS2TypoAscender", "<integer>900</integer>"),
        ("openTypeOS2TypoDescender", "<integer>-300</integer>"),
        ("openTypeOS2TypoLineGap", "<integer>50</integer>"),
        ("openTypeOS2WinAscent", "<integer>1000</integer>"),
        ("openTypeOS2WinDescent", "<integer>400</integer>"),
        ("openTypeHheaAscender", "<integer>950</integer>"),
        ("openTypeHheaDescender", "<integer>-320</integer>"),
        ("openTypeOS2Selection", "<array><integer>7</integer></array>"),
    ];
    let body: String = info.iter().map(|(k, v)| format!("<key>{k}</key>{v}")).collect();
    std::fs::write(ufo.join("fontinfo.plist"), plist(&format!("<dict>{body}</dict>"))).unwrap();
    std::fs::write(ufo.join("layercontents.plist"), plist("<array><array><string>public.default</string><string>glyphs</string></array></array>"))
        .unwrap();
    let mut contents = String::new();
    let mut order = String::new();
    for (name, file, width, code, outline) in METRIC_GLYPHS {
        contents.push_str(&format!("<key>{name}</key><string>{file}.glif</string>"));
        order.push_str(&format!("<string>{name}</string>"));
        let unicode = code.map(|c| format!("<unicode hex=\"{c:04X}\"/>")).unwrap_or_default();
        std::fs::write(
            glyphs.join(format!("{file}.glif")),
            format!(
                "<?xml version='1.0' encoding='UTF-8'?>\n<glyph name=\"{name}\" format=\"2\"><advance width=\"{width}\"/>{unicode}<outline>{outline}</outline></glyph>\n"
            ),
        )
        .unwrap();
    }
    std::fs::write(glyphs.join("contents.plist"), plist(&format!("<dict>{contents}</dict>"))).unwrap();
    std::fs::write(ufo.join("lib.plist"), plist(&format!("<dict><key>public.glyphOrder</key><array>{order}</array></dict>"))).unwrap();
    std::fs::write(ufo.join("kerning.plist"), plist("<dict><key>O</key><dict><key>d</key><integer>-30</integer></dict></dict>")).unwrap();
    std::fs::write(ufo.join("features.fea"), features).unwrap();
    ufo
}

/// Builds the metrics UFO with `features` as a project compile, opened as the command line opens
/// a project compile's source (composites as the file places them).
fn build_metrics(name: &str, features: &str) -> Vec<u8> {
    let dir = temp(name);
    let ufo = write_metrics_ufo(&dir, features);
    let mut s = Session::new();
    s.settings_mut().set("user.disableAutomaticAlignmentForImportedFiles", json!(true)).unwrap();
    s.execute("file.open", &json!({ "path": ufo })).unwrap();
    let out = dir.join("out.otf");
    s.execute("font.compile", &json!({"formats": ["otf"], "destination": out, "timestamp": 0, "projectOptions": []})).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    bytes
}

/// The points of glyph `name` as the outline draws them, contour by contour.
fn contours(bytes: &[u8], name: &str) -> Vec<Vec<(f32, f32)>> {
    use skrifa::instance::{LocationRef, Size};
    use skrifa::outline::{DrawSettings, OutlinePen};
    #[derive(Default)]
    struct Pen(Vec<Vec<(f32, f32)>>);
    impl OutlinePen for Pen {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.push(vec![(x, y)]);
        }
        fn line_to(&mut self, x: f32, y: f32) {
            if let Some(c) = self.0.last_mut() {
                c.push((x, y));
            }
        }
        fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) {
            self.line_to(x, y);
        }
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) {
            self.line_to(x, y);
        }
        fn close(&mut self) {}
    }
    let font = FontRef::new(bytes).unwrap();
    let gid = font.glyph_names().iter().find(|(_, n)| n.as_str() == name).unwrap().0;
    let glyph = font.outline_glyphs().get(gid).unwrap();
    let mut pen = Pen::default();
    glyph.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).unwrap();
    pen.0
}

#[test]
fn a_project_compile_takes_metrics_names_and_outlines_as_the_reference_does() {
    let bytes = build_metrics("metrics", "languagesystem DFLT dflt;\nfeature liga { sub d d by O; } liga;\n");
    let font = FontRef::new(&bytes).unwrap();
    let os2 = font.os2().unwrap();
    // From the full name, not the source's 200; neither bold nor italic nor 400: no style bits.
    assert_eq!((os2.us_weight_class(), os2.us_width_class(), os2.fs_selection().bits()), (300, 5, 0));
    // d is 761 high, O 712 less 13 of overshoot; the font box runs from -231 to 950.
    assert_eq!((os2.s_typo_ascender(), os2.s_typo_descender(), os2.s_typo_line_gap()), (761, -239, 200));
    assert_eq!((os2.us_win_ascent(), os2.us_win_descent()), (950, 231));
    assert_eq!((os2.sx_height(), os2.s_cap_height()), (Some(500), Some(699)));
    let hhea = font.hhea().unwrap();
    assert_eq!((hhea.ascender().to_i16(), hhea.descender().to_i16(), hhea.line_gap().to_i16()), (761, -239, 200));
    assert_eq!(font.head().unwrap().font_revision().to_f64(), 1.0);
    let n = names(&bytes);
    assert!(win(&n, 5).unwrap().starts_with("Version 1.000"), "{n:?}");
    for id in [0, 7, 9, 14] {
        assert!(win(&n, id).is_none(), "name {id} is not written: {n:?}");
    }
    assert!(font.gpos().is_err(), "the kerning is not used");
    // Coordinates keep two decimals; the composite keeps its own advance; overlaps stay.
    let d = contours(&bytes, "d");
    assert!(d[0].iter().any(|(x, y)| (x - 140.26).abs() < 0.001 && (y - 760.4).abs() < 0.001), "{d:?}");
    let gid = font.glyph_names().iter().find(|(_, n)| n.as_str() == "Oacute").unwrap().0;
    assert_eq!(font.hmtx().unwrap().advance(gid), Some(720));
    assert_eq!(contours(&bytes, "bar").len(), 2);
}

#[test]
fn the_feature_code_overrides_what_the_project_compile_works_out() {
    let code = "languagesystem DFLT dflt;\ntable head { FontRevision 2.5; } head;\ntable OS/2 { WeightClass 400; } OS/2;\ntable hhea { Ascender 900; } hhea;\n";
    let bytes = build_metrics("overrides", code);
    let font = FontRef::new(&bytes).unwrap();
    let os2 = font.os2().unwrap();
    assert_eq!((os2.us_weight_class(), os2.fs_selection().bits()), (400, 0x40), "a weight of 400 makes the font Regular");
    let hhea = font.hhea().unwrap();
    assert_eq!((hhea.ascender().to_i16(), hhea.descender().to_i16(), hhea.line_gap().to_i16()), (900, -239, 200));
    assert_eq!(font.head().unwrap().font_revision().to_f64(), 2.5);
    assert!(win(&names(&bytes), 5).unwrap().starts_with("Version 2.500"));
}
