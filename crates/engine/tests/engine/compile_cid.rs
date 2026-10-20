//! `font.compile` of a CID-keyed font (M12.41): a CID-keyed CFF whose cmap comes from the
//! Unicode CMap file the request names; a CMap file that cannot be read writes nothing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;

use fontcraft_engine::Session;
use serde_json::{Value, json};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, MetadataProvider};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-compile-cid-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A UTF-32 CMap of the made-up `Test-Identity` collection: A is CID 2 and a is CID 28, as the
/// Identity conversion numbers them (`.notdef`, space, then A to Z, a to z).
const CMAP: &str = "/CIDSystemInfo 3 dict dup begin /Registry (Test) def /Ordering (Identity) def /Supplement 0 def end def\n\
/CMapName /UniTest-UTF32-H def\n1 begincidchar\n<00000041> 2\nendcidchar\n1 begincidrange\n<00000061> <00000062> 28\nendcidrange\n";

/// The first operator of the CFF table's Top DICT, as one byte or 1200 + the escaped byte.
fn first_top_operator(cff: &[u8]) -> u16 {
    let index_end = |at: usize| -> (usize, usize) {
        let count = usize::from(u16::from_be_bytes([cff[at], cff[at + 1]]));
        let size = usize::from(cff[at + 2]);
        let off = |i: usize| cff[at + 3 + i * size..at + 3 + (i + 1) * size].iter().fold(0usize, |a, b| a << 8 | usize::from(*b));
        let base = at + 2 + (count + 1) * size;
        (base + off(0), base + off(count))
    };
    let (_, names_end) = index_end(usize::from(cff[2]));
    let (top, _) = index_end(names_end);
    let mut i = top;
    loop {
        match cff[i] {
            12 => return 1200 + u16::from(cff[i + 1]),
            b @ 0..=21 => return u16::from(b),
            28 => i += 3,
            29 => i += 5,
            32..=246 => i += 1,
            _ => i += 2,
        }
    }
}

#[test]
fn a_cid_keyed_font_compiles_with_its_cmap_file_and_a_bad_cmap_file_writes_nothing() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"familyName": "Cid Sans"}));
    run(&mut s, "cid.convertToCid", json!({"registry": "Test", "ordering": "Identity", "supplement": 0}));
    let dir = temp("ok");
    let cmap = dir.join("UniTest-UTF32-H");
    std::fs::write(&cmap, CMAP).unwrap();
    let out = dir.join("CidSans.otf");
    let r = run(
        &mut s,
        "font.compile",
        json!({"formats": ["otf"], "destination": out.to_str().unwrap(), "cidCmap": cmap.to_str().unwrap(), "versionStamp": false}),
    );
    assert_eq!(r["fonts"].as_array().unwrap().len(), 1, "{r}");
    let bytes = std::fs::read(&out).unwrap();
    let font = FontRef::new(&bytes).unwrap();
    let cff = font.table_data(skrifa::raw::types::Tag::new(b"CFF ")).unwrap();
    assert_eq!(first_top_operator(cff.as_bytes()), 1230, "a CID-keyed Top DICT starts with ROS");
    let charmap = font.charmap();
    assert_eq!(charmap.map('A').map(|g| g.to_u32()), Some(2));
    assert_eq!(charmap.map('b').map(|g| g.to_u32()), Some(29));
    assert_eq!(charmap.map('B'), None, "only the CMap's code points are encoded");
    assert_eq!(font.post().unwrap().version(), skrifa::raw::types::Version16Dot16::VERSION_3_0);

    let bad_dir = temp("bad");
    let bad = bad_dir.join("UniBad-UTF32-H");
    std::fs::write(&bad, "/CMapName /UniBad-UTF32-H def 1 begincidrange <41> 2 endcidrange").unwrap();
    let dest = bad_dir.join("out");
    let e = s
        .execute("font.compile", &json!({"formats": ["otf"], "destination": dest.to_str().unwrap(), "cidCmap": bad.to_str().unwrap()}))
        .map(|r| r.to_string())
        .unwrap_or_else(|e| e.to_string());
    assert!(e.contains("Unicode CMap cannot be read"), "{e}");
    assert!(!dest.exists() || std::fs::read_dir(&dest).unwrap().next().is_none(), "a failing compile writes nothing");
    let e = s
        .execute("font.compile", &json!({"formats": ["otf"], "destination": dest.to_str().unwrap(), "cidMacScript": 40000}))
        .expect_err("a script id beyond 32767")
        .to_string();
    assert!(e.contains("cidMacScript"), "{e}");
}

/// A UTF-32 CMap of the made-up `Test-Order` collection that puts B before A.
const ORDER_CMAP: &str = "/CIDSystemInfo 3 dict dup begin /Registry (Test) def /Ordering (Order) def /Supplement 0 def end def\n\
/CMapName /UniTest-UTF32-H def\n2 begincidchar\n<00000042> 1\n<00000041> 2\nendcidchar\n";

#[test]
fn the_compile_plan_gives_the_cid_order_the_compile_writes() {
    let dir = temp("plan");
    let cmap = dir.join("UniTest-UTF32-H");
    std::fs::write(&cmap, ORDER_CMAP).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"familyName": "Cid Sans"}));
    run(&mut s, "cid.convertToCid", json!({"registry": "Test", "ordering": "Order", "supplement": 0, "cmap": cmap.to_str().unwrap()}));
    let plan = run(&mut s, "font.compilePlan", json!({"formats": ["otf"], "sections": ["glyphOrder"]}));
    let order: Vec<&str> = plan["glyphOrder"].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect();
    assert_eq!(order.get(..4), Some(&[".notdef", "B", "A", "space"][..]), "CID order, the unmapped glyphs after the highest CID");

    let out = dir.join("CidSans.otf");
    run(&mut s, "font.compile", json!({"formats": ["otf"], "destination": out.to_str().unwrap()}));
    let bytes = std::fs::read(&out).unwrap();
    let font = FontRef::new(&bytes).unwrap();
    assert_eq!(usize::from(font.maxp().unwrap().num_glyphs()), order.len());
    // The CID-keyed font has no glyph names; its cmap shows which glyph each glyph id holds.
    for (c, gid) in font.charmap().mappings() {
        let Some(ch) = char::from_u32(c).filter(|ch| ch.is_ascii_alphabetic() || *ch == ' ') else { continue };
        let name = if ch == ' ' { "space".to_string() } else { ch.to_string() };
        assert_eq!(order.get(gid.to_u32() as usize).copied(), Some(name.as_str()), "U+{c:04X}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A format 3 source with the `ROS` parameter (`{ROS}` is replaced): `cid00005` and `cid00100`
/// name their CIDs, `uni4E00` and `A` have code points only.
const ROS_SOURCE: &str = r#"{
.formatVersion = 3;
customParameters = (
{
name = ROS;
value = "{ROS}";
}
);
familyName = "Ros Test";
fontMaster = (
{
id = m01;
}
);
glyphs = (
{
glyphname = cid00005;
layers = (
{
layerId = m01;
shapes = (
{
closed = 1;
nodes = (
(100,0,l),
(500,0,l),
(500,700,l),
(100,700,l)
);
}
);
width = 1000;
}
);
},
{
glyphname = cid00100;
layers = (
{
layerId = m01;
width = 1000;
}
);
},
{
glyphname = uni4E00;
layers = (
{
layerId = m01;
width = 1000;
}
);
unicode = 19968;
},
{
glyphname = A;
layers = (
{
layerId = m01;
width = 600;
}
);
unicode = 65;
}
);
unitsPerEm = 1000;
}
"#;

const ROS_PARAMETER: &str = "customParameters = (\n{\nname = ROS;\nvalue = \"{ROS}\";\n}\n);\n";

/// A UTF-32 CMap of `Adobe-Japan1` that gives U+4E00 CID 1200.
const JAPAN1_CMAP: &str = "/CIDSystemInfo 3 dict dup begin /Registry (Adobe) def /Ordering (Japan1) def /Supplement 6 def end def\n\
/CMapName /UniJIS-UTF32-H def\n1 begincidchar\n<00004E00> 1200\nendcidchar\n";

/// Compiles `source` from a `.glyphs` file to an OTF; returns the font's bytes and the TTX dump of
/// its CFF table.
fn compile_glyphs_source(name: &str, source: &str, extra: Value) -> (Vec<u8>, String) {
    let dir = temp(name);
    let path = dir.join("RosTest.glyphs");
    std::fs::write(&path, source).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path.to_str().unwrap()}));
    let out = dir.join("RosTest.otf");
    let mut p = json!({"formats": ["otf"], "destination": out.to_str().unwrap(), "versionStamp": false});
    if let (Some(p), Value::Object(extra)) = (p.as_object_mut(), extra) {
        p.extend(extra);
    }
    run(&mut s, "font.compile", p);
    let bytes = std::fs::read(&out).unwrap();
    let sfnt = fontcraft_tables::sfnt::read(&bytes, 0).unwrap();
    let ttx = fontcraft_tables::ttx::dump(&sfnt, &fontcraft_tables::ttx::DumpOptions { tables: vec![*b"CFF "], ..Default::default() }).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    (bytes, ttx)
}

/// glyph id → CID of a CID-keyed font's charset.
fn cids(bytes: &[u8]) -> Vec<u16> {
    use skrifa::raw::ps::cff::CffFontRef;
    let font = FontRef::new(bytes).unwrap();
    let table = font.table_data(skrifa::raw::types::Tag::new(b"CFF ")).unwrap();
    let cff = CffFontRef::new_cff(table.as_bytes(), 0, None).unwrap();
    assert!(cff.is_cid());
    let charset = cff.charset().unwrap();
    let count = u32::from(font.maxp().unwrap().num_glyphs());
    (0..count).map(|g| charset.string_id(skrifa::GlyphId::new(g)).unwrap().to_u16()).collect()
}

#[test]
fn the_ros_parameter_exports_a_cid_keyed_font_with_the_cids_the_glyph_names_give() {
    let (bytes, ttx) = compile_glyphs_source("ros", &ROS_SOURCE.replace("{ROS}", "Adobe-Japan1-6"), json!({}));
    assert!(ttx.contains(r#"<ROS Registry="Adobe" Order="Japan1" Supplement="6"/>"#), "{ttx}");
    // .notdef, the named CIDs, then the glyphs the collection does not cover, in glyph order.
    assert_eq!(cids(&bytes), [0, 5, 100, 101, 102]);
    let font = FontRef::new(&bytes).unwrap();
    assert_eq!(font.charmap().map('A').map(|g| g.to_u32()), Some(4), "without a CMap file the source's code points are encoded");
    assert_eq!(font.post().unwrap().version(), skrifa::raw::types::Version16Dot16::VERSION_3_0);

    // A Unicode CMap gives a nice-named glyph its CID through its code point.
    let dir = temp("ros-cmap");
    let cmap = dir.join("UniJIS-UTF32-H");
    std::fs::write(&cmap, JAPAN1_CMAP).unwrap();
    let (bytes, _) =
        compile_glyphs_source("ros-with-cmap", &ROS_SOURCE.replace("{ROS}", "Adobe-Japan1-6"), json!({"cidCmap": cmap.to_str().unwrap()}));
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(cids(&bytes), [0, 5, 100, 1200, 1201]);
    let font = FontRef::new(&bytes).unwrap();
    assert_eq!(font.charmap().map('\u{4E00}').map(|g| g.to_u32()), Some(3));
    assert_eq!(font.charmap().map('A'), None, "only the CMap's code points are encoded");

    // The Identity ordering covers no glyph: CIDs follow the glyph order after the named ones.
    let (bytes, ttx) = compile_glyphs_source("ros-identity", &ROS_SOURCE.replace("{ROS}", "Adobe-Identity-0"), json!({}));
    assert!(ttx.contains(r#"<ROS Registry="Adobe" Order="Identity" Supplement="0"/>"#), "{ttx}");
    assert_eq!(cids(&bytes), [0, 5, 100, 101, 102]);
}

#[test]
fn without_the_ros_parameter_cid_names_export_name_keyed() {
    let source = ROS_SOURCE.replace(ROS_PARAMETER, "");
    assert_ne!(source, ROS_SOURCE);
    let (bytes, ttx) = compile_glyphs_source("no-ros", &source, json!({}));
    assert!(!ttx.contains("<ROS "), "{ttx}");
    let font = FontRef::new(&bytes).unwrap();
    assert_eq!(font.post().unwrap().version(), skrifa::raw::types::Version16Dot16::VERSION_2_0);
    let names: Vec<String> = (0..5).filter_map(|g| font.glyph_names().get(skrifa::GlyphId::new(g)).map(|n| n.to_string())).collect();
    assert_eq!(names, [".notdef", "cid00005", "cid00100", "uni4E00", "A"]);
}
