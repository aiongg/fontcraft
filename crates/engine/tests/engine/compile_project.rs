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

#[test]
fn a_confined_project_compile_reads_no_derived_path_outside_the_read_root() {
    use fontcraft_engine::file_access::{AutomationRoots, confine};
    let base = temp("confined");
    let inside = base.join("in");
    let outside = base.join("outside");
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let ufo = write_ufo(&inside);
    // Not a FontMenuNameDB: had it been read, the compile would fail quoting this line.
    let hidden = outside.join("FontMenuNameDB");
    std::fs::write(&hidden, "hidden words\n").unwrap();
    let mut s = Session::new();
    s.execute("file.open", &json!({ "path": ufo })).unwrap();
    let roots = AutomationRoots::new(Some(&inside), Some(&inside)).unwrap().unwrap();
    let out = inside.join("out.otf");
    let fpr = inside.join("current.fpr");
    let absolute = hidden.display().to_string();
    for (key, value) in
        [("FontMenuDBPath", "../outside/FontMenuNameDB"), ("FontMenuDBPath", absolute.as_str()), ("GlyphAliasDBPath", "../outside/FontMenuNameDB")]
    {
        std::fs::write(&fpr, format!("{key} {value}\n")).unwrap();
        let params = json!({"formats": ["otf"], "destination": out, "projectFile": fpr});
        let e = confine(Some(&roots), || s.execute("font.compile", &params)).unwrap_err().to_string();
        assert!(e.contains(key) && e.contains("outside the read root"), "{key} {value}: {e}");
        assert!(!e.contains("hidden words"), "{e}");
        assert!(!out.exists());
    }

    // The defaults search skips a FontMenuNameDB above the read root, as if it were missing.
    std::fs::write(base.join("FontMenuNameDB"), "[Synth-Regular]\n\tf=Synth Pro\n\ts=Light Italic\n\tl=Synth Pro Light\n").unwrap();
    let params = json!({"formats": ["otf"], "destination": out, "projectDefaults": true, "timestamp": 0});
    confine(Some(&roots), || s.execute("font.compile", &params)).unwrap();
    let n = names(&std::fs::read(&out).unwrap());
    assert_eq!((win(&n, 1), win(&n, 16)), (Some("Synth"), None));
    std::fs::remove_file(&out).unwrap();
    s.execute("font.compile", &params).unwrap();
    assert_eq!(win(&names(&std::fs::read(&out).unwrap()), 16), Some("Synth Pro"), "found without roots");
    std::fs::remove_dir_all(&base).unwrap();
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

/// The points of glyph `name` as its charstring states them, contour by contour, fractions kept
/// (the outline drawing API drops the fractions of CFF coordinates).
fn contours(bytes: &[u8], name: &str) -> Vec<Vec<(f32, f32)>> {
    use skrifa::raw::ps::cff::CffFontRef;
    use skrifa::raw::ps::cs::CommandSink;
    use skrifa::raw::types::{Fixed, Tag};
    #[derive(Default)]
    struct Pen(Vec<Vec<(f32, f32)>>);
    impl CommandSink for Pen {
        fn move_to(&mut self, x: Fixed, y: Fixed) {
            self.0.push(vec![(x.to_f32(), y.to_f32())]);
        }
        fn line_to(&mut self, x: Fixed, y: Fixed) {
            if let Some(c) = self.0.last_mut() {
                c.push((x.to_f32(), y.to_f32()));
            }
        }
        fn curve_to(&mut self, _: Fixed, _: Fixed, _: Fixed, _: Fixed, x: Fixed, y: Fixed) {
            self.line_to(x, y);
        }
        fn close(&mut self) {}
    }
    let font = FontRef::new(bytes).unwrap();
    let gid = font.glyph_names().iter().find(|(_, n)| n.as_str() == name).unwrap().0;
    let table = font.table_data(Tag::new(b"CFF ")).unwrap();
    let cff = CffFontRef::new_cff(table.as_bytes(), 0, None).unwrap();
    let subfont = cff.subfont(0, &[]).unwrap();
    let mut pen = Pen::default();
    cff.evaluate_charstring(&subfont, gid, &[], &mut pen).unwrap();
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

/// The `OS/2` version, `fsSelection` and length of a built font.
fn os2(bytes: &[u8]) -> (u16, u16, usize) {
    let font = FontRef::new(bytes).unwrap();
    let table = font.table_data(skrifa::Tag::new(b"OS/2")).unwrap();
    let b = table.as_bytes();
    (u16::from_be_bytes([b[0], b[1]]), u16::from_be_bytes([b[62], b[63]]), b.len())
}

/// Each case's version, `fsSelection` and table length are what makeotf 5.0.1's addfeatures
/// wrote for a Regular font with the same `-osbOn`, `-osbOff` and `-osv` options; the checks of
/// bits 7 to 9 are makeotf.py's.
#[test]
fn fs_selection_bits_and_the_os2_version_follow_the_reference() {
    let cases = [
        (json!({"projectOptions": []}), (3, 0x40, 96)),
        (json!({"os2BitsOn": [7, 8, 9]}), (4, 0x3C0, 96)),
        (json!({"os2BitsOff": [6]}), (3, 0, 96)),
        (json!({"os2Version": 2}), (2, 0x40, 96)),
        (json!({"os2Version": 1}), (1, 0x40, 96)),
        (json!({"os2Version": 6}), (6, 0x40, 100)),
        // A bit from 7 up forces version 4, whatever version is asked for.
        (json!({"os2BitsOn": [7], "os2BitsOff": [8, 9], "os2Version": 2}), (4, 0xC0, 96)),
        // Bit 15 does not take effect, and the bits after it start the mask again.
        (json!({"os2BitsOn": [15, 7], "os2BitsOff": [8, 9]}), (4, 0xC0, 96)),
        // Sorted as text, 0 comes first, so bit 10 needs no values for bits 7 to 9.
        (json!({"os2BitsOn": [0, 10]}), (4, 0x441, 96)),
    ];
    for (i, (extra, want)) in cases.into_iter().enumerate() {
        let dir = temp(&format!("os2-{i}"));
        let (bytes, _) = build(&dir, extra.clone());
        assert_eq!(os2(&bytes), want, "{extra}");
    }
    let dir = temp("os2-refused");
    let ufo = write_ufo(&dir);
    let mut s = Session::new();
    s.execute("file.open", &json!({ "path": ufo })).unwrap();
    for (params, says) in [
        (json!({"os2BitsOn": [8]}), "no value was given for bit 7, 9"),
        (json!({"os2BitsOff": [0]}), "bit 0 cannot be turned off"),
        (json!({"os2BitsOn": [16]}), "bit 16 cannot be turned on"),
    ] {
        let mut p = params.clone();
        p["formats"] = json!(["otf"]);
        p["destination"] = json!(dir.join("no.otf"));
        let e = s.execute("font.compile", &p).unwrap_err().to_string();
        assert!(e.contains(says), "{params}: {e}");
        assert!(!dir.join("no.otf").exists());
    }
}

/// The project file's paths are relative to its folder: the feature file (with its includes),
/// the variation sequences and the output file; the licence code ends name ID 3.
#[test]
fn a_project_file_names_the_feature_file_sequences_licence_and_output() {
    let dir = temp("project-paths");
    let ufo = write_ufo(&dir);
    let project = dir.join("project");
    std::fs::create_dir_all(project.join("fea")).unwrap();
    std::fs::create_dir_all(dir.join("fonts")).unwrap();
    std::fs::write(project.join("fea/alt.fea"), "include(rev.fea);\n").unwrap();
    std::fs::write(project.join("fea/rev.fea"), "table head { FontRevision 2.5; } head;\n").unwrap();
    std::fs::write(project.join("seq.uvs"), "# sequences\n0041 FE00; B\n0042 FE00; B\n0041 FE01; nothere\n").unwrap();
    let fpr = project.join("current.fpr");
    std::fs::write(
        &fpr,
        "InputFontPath ../font.ufo\nOutputFontPath ../fonts/Out.otf\nFeaturePath fea/alt.fea\nUVSPath seq.uvs\nLicenseCode XYZ\nkSetfsSelectionBitsOn 7\n",
    )
    .unwrap();
    let mut s = Session::new();
    s.execute("file.open", &json!({ "path": ufo })).unwrap();
    let result = s.execute("font.compile", &json!({"formats": ["otf"], "projectFile": fpr, "timestamp": 0})).unwrap();
    let warnings = result["warnings"].to_string();
    assert!(!warnings.contains("InputFontPath"), "the open source is the project's input: {warnings}");
    let bytes = std::fs::read(dir.join("fonts/Out.otf")).unwrap();
    let n = names(&bytes);
    assert!(win(&n, 5).unwrap().starts_with("Version 2.500"), "the feature file and its include are read: {n:?}");
    assert!(win(&n, 3).unwrap().ends_with(";XYZ"), "{n:?}");
    // makeotf's project file bits count as giving 7, 8 and 9 a value.
    assert_eq!(os2(&bytes), (4, 0xC0, 96));
    let font = FontRef::new(&bytes).unwrap();
    let charmap = font.charmap();
    let b = charmap.map('B').unwrap();
    assert_eq!(charmap.map_variant(0x41u32, 0xFE00u32), Some(skrifa::charmap::MapVariant::Variant(b)));
    assert_eq!(charmap.map_variant(0x42u32, 0xFE00u32), Some(skrifa::charmap::MapVariant::UseDefault));
    assert_eq!(charmap.map_variant(0x41u32, 0xFE01u32), None);
    assert!(result.to_string().contains("nothere"), "the missing glyph is reported: {result}");

    // A destination given wins over OutputFontPath, with a note.
    let out = dir.join("given.otf");
    let result = s.execute("font.compile", &json!({"formats": ["otf"], "projectFile": fpr, "destination": out, "timestamp": 0})).unwrap();
    assert!(out.exists());
    assert!(result["warnings"].to_string().contains("OutputFontPath is not used"), "{result}");
}

/// makeotf 5.0.1 with `-lic ABC` wrote `1.000;UKWN;Synth-Regular;ABC` where it otherwise wrote
/// `1.000;UKWN;Synth-Regular`.
#[test]
fn the_licence_code_ends_the_unique_identifier() {
    let dir = temp("licence");
    let (plain, _) = build(&dir, json!({"projectOptions": []}));
    let (coded, _) = build(&dir, json!({"licenseCode": "ABC"}));
    let id3 = win(&names(&plain), 3).unwrap().to_string();
    assert_eq!(win(&names(&coded), 3), Some(format!("{id3};ABC").as_str()));
}
