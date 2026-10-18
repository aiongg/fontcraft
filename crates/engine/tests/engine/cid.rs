//! CID-keyed fonts: converting with the Identity ordering and with a CMap file, flattening back,
//! editing sub-fonts, and the CID data surviving `.glyphs` and UFO. Refused requests change
//! nothing.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_model::cid::{CidFont, is_cid_keyed};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Runs a command that must fail and checks that the font is the same object as before.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {p}: a refused command changed the font");
    e
}

fn session(family: &str) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"familyName": family}));
    s
}

fn state(s: &Session) -> CidFont {
    CidFont::read(s.font().unwrap()).unwrap().expect("CID-keyed")
}

fn names(s: &Session) -> Vec<String> {
    s.font().unwrap().glyphs.iter().map(|g| g.name.clone()).collect()
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-cid-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A UTF-32 CMap of a made-up collection: A to C are CIDs 10 to 12, a is 20, space is 1.
const CMAP: &str = "%!PS-Resource-CMap\n/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
/CIDSystemInfo 3 dict dup begin\n  /Registry (Test) def\n  /Ordering (Latin1) def\n  /Supplement 2 def\nend def\n\
/CMapName /UniTest-UTF32-H def\n/CMapType 1 def\n1 begincodespacerange\n<00000000> <0010FFFF>\nendcodespacerange\n\
2 begincidrange\n<00000041> <00000043> 10\n<00000061> <00000061> 20\nendcidrange\n\
1 begincidchar\n<00000020> 1\nendcidchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n";

/// The same collection, but CID 10 is U+0391 and CID 20 is U+03B1.
const GREEK_CMAP: &str = "/CIDSystemInfo 3 dict dup begin /Registry (Test) def /Ordering (Latin1) def /Supplement 2 def end def\n\
/CMapName /UniTestGreek-UTF32-H def\n2 begincidchar\n<00000391> 10\n<000003B1> 20\nendcidchar\n";

#[test]
fn cid_convert_to_cid_with_identity_follows_the_glyph_order_and_cid_flatten_gives_the_font_back() {
    let mut s = session("Cid Sans");
    let plain = s.doc().unwrap().font.clone();
    let order = names(&s);
    let r = run(&mut s, "cid.convertToCid", json!({"registry": "Test", "ordering": "Identity", "supplement": 0}));
    assert_eq!(r["ros"], "Test-Identity-0");
    assert_eq!(r["cids"], 54, ".notdef, space and 52 letters");
    assert_eq!(r["highest"], 53);
    assert_eq!(r["appended"], json!([]));
    let cid = state(&s);
    assert_eq!(cid.cid_of(".notdef"), Some(0));
    assert_eq!(cid.cid_of("space"), Some(1));
    assert_eq!(cid.cid_of("A"), Some(2));
    assert_eq!(cid.subfonts.len(), 1);
    assert_eq!(cid.subfonts[0].name, "CidSans");
    assert_eq!(cid.subfonts[0].glyphs.len(), 54);
    assert_eq!(cid.font_name.as_deref(), Some("CidSans"));
    assert_eq!(names(&s), order, "converting keeps the glyph order");

    // Converting twice, flattening or editing an ordinary font are refused.
    refused(&mut s, "cid.convertToCid", json!({"registry": "Test", "ordering": "Identity", "supplement": 0}));

    let r = run(&mut s, "cid.flatten", json!({}));
    assert_eq!(r["glyphs"], 54);
    assert_eq!(r["subfonts"], 1);
    let font = s.font().unwrap();
    assert!(!is_cid_keyed(font));
    assert!(font.groups.is_empty());
    assert_eq!(names(&s), order);
    assert!(font.masters[0].user_data.is_empty(), "the CID keys are gone");

    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &plain), "each command is one undo step");
    refused(&mut s, "cid.flatten", json!({}));
    refused(&mut s, "cid.editSubfonts", json!({"show": "CidSans"}));
}

#[test]
fn cid_convert_to_cid_with_a_cmap_maps_code_points_and_appends_the_rest() {
    let dir = temp_dir("cmap");
    let cmap = dir.join("UniTest-UTF32-H");
    std::fs::write(&cmap, CMAP).unwrap();
    let greek = dir.join("UniTestGreek-UTF32-H");
    std::fs::write(&greek, GREEK_CMAP).unwrap();
    let mut s = session("Cid Sans");

    // Another collection's ordering without a CMap, or with a CMap of another collection.
    let e = refused(&mut s, "cid.convertToCid", json!({"registry": "Test", "ordering": "Latin1", "supplement": 2}));
    assert!(e.contains("cmap"), "{e}");
    let e = refused(&mut s, "cid.convertToCid", json!({"registry": "Test", "ordering": "Han1", "supplement": 2, "cmap": cmap}));
    assert!(e.contains("Test-Latin1"), "{e}");
    refused(&mut s, "cid.convertToCid", json!({"registry": "Te-st", "ordering": "Identity", "supplement": 0}));
    refused(&mut s, "cid.convertToCid", json!({"registry": "Test", "ordering": "Identity"}));

    let r = run(&mut s, "cid.convertToCid", json!({"registry": "Test", "ordering": "Latin1", "supplement": 2, "cmap": cmap}));
    assert_eq!(r["mapped"], 5, "space, A, B, C and a");
    let appended: Vec<&str> = r["appended"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(appended.len(), 23 + 25);
    assert_eq!(appended[..2], ["D", "E"]);
    assert_eq!(r["highest"], 20 + 48);
    let cid = state(&s);
    let got: Vec<Option<u32>> = [".notdef", "space", "A", "B", "C", "a", "D", "b"].iter().map(|g| cid.cid_of(g)).collect();
    assert_eq!(got, [Some(0), Some(1), Some(10), Some(11), Some(12), Some(20), Some(21), Some(44)]);

    // Flattening by another CMap of the collection: glyphs in CID order, A and a re-encoded.
    let r = run(&mut s, "cid.flatten", json!({"cmap": greek}));
    assert_eq!(r["encoded"], 2);
    assert_eq!(names(&s)[..7], [".notdef", "space", "A", "B", "C", "a", "D"]);
    let font = s.font().unwrap();
    assert_eq!(font.glyph("A").unwrap().unicodes, [0x391]);
    assert_eq!(font.glyph("a").unwrap().unicodes, [0x3B1]);
    assert_eq!(font.glyph("B").unwrap().unicodes, [0x42], "a glyph the CMap does not reach keeps its code points");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn cid_edit_subfonts_inserts_removes_shows_and_sets_hinting() {
    let dir = temp_dir("subfonts");
    let mut other = session("Other Han");
    let other_path = dir.join("Other.glyphs");
    run(&mut other, "file.saveAs", json!({"path": other_path, "version": 3}));

    let mut s = session("Cid Sans");
    run(&mut s, "cid.convertToCid", json!({"registry": "Test", "ordering": "Identity", "supplement": 0}));

    refused(&mut s, "cid.editSubfonts", json!({}));
    refused(&mut s, "cid.editSubfonts", json!({"show": "CidSans", "supplement": 1}));
    refused(&mut s, "cid.editSubfonts", json!({"remove": "CidSans"}));
    refused(&mut s, "cid.editSubfonts", json!({"insert": "blank", "name": "has space"}));
    refused(&mut s, "cid.editSubfonts", json!({"insert": "blank", "name": "CidSans"}));

    let r = run(&mut s, "cid.editSubfonts", json!({"insert": "blank", "name": "Dingbats"}));
    assert_eq!((r["subfont"].as_str(), r["index"].as_u64()), (Some("Dingbats"), Some(1)));
    assert_eq!(r["glyphs"], json!([]));

    // Every glyph of the other font but .notdef clashes: renamed, code points dropped.
    let r = run(&mut s, "cid.editSubfonts", json!({"insert": other_path}));
    assert_eq!(r["subfont"], "OtherHan");
    assert_eq!(r["index"], 2);
    assert_eq!(r["renamed"].as_array().unwrap().len(), 53);
    assert_eq!(r["renamed"][1], json!({"from": "A", "to": "A.001"}));
    assert_eq!(r["droppedCodepoints"], 53);
    let cid = state(&s);
    assert_eq!(cid.cid_of("space.001"), Some(54), "inserted glyphs follow the highest CID");
    assert_eq!(cid.subfont_of("A.001"), Some(2));
    assert!(s.font().unwrap().glyph("A.001").unwrap().unicodes.is_empty());

    let r = run(&mut s, "cid.editSubfonts", json!({"show": "OtherHan"}));
    assert_eq!(r["glyphs"].as_array().unwrap().len(), 53);
    assert_eq!(r["glyphs"][0], "space.001");

    let r = run(&mut s, "cid.editSubfonts", json!({"supplement": 3}));
    assert_eq!(r["ros"], "Test-Identity-3");
    assert_eq!(state(&s).ros.supplement, 3);

    let r = run(
        &mut s,
        "cid.editSubfonts",
        json!({"private": {"subfont": "OtherHan", "values": {"postscriptBlueValues": [-12, 0, 880, 892], "postscriptBlueScale": 0.039625}}}),
    );
    assert_eq!(r["private"], json!({"postscriptBlueValues": [-12, 0, 880, 892], "postscriptBlueScale": 0.039625}));
    refused(&mut s, "cid.editSubfonts", json!({"private": {"subfont": "OtherHan", "values": {"BlueValues": [0]}}}));
    refused(&mut s, "cid.editSubfonts", json!({"private": {"subfont": "OtherHan", "values": {"postscriptBlueFuzz": "x"}}}));

    let count = s.font().unwrap().glyphs.len();
    let r = run(&mut s, "cid.editSubfonts", json!({"remove": "OtherHan"}));
    assert_eq!(r["removed"].as_array().unwrap().len(), 53);
    assert_eq!(s.font().unwrap().glyphs.len(), count - 53);
    let cid = state(&s);
    assert_eq!(cid.subfonts.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["CidSans", "Dingbats"]);
    assert_eq!(cid.cid_of("A.001"), None);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn cid_keyed_data_survives_glyphs_and_ufo() {
    let dir = temp_dir("formats");
    let make = || {
        let mut s = session("Cid Sans");
        run(&mut s, "cid.convertToCid", json!({"registry": "Test", "ordering": "Identity", "supplement": 0}));
        run(&mut s, "cid.editSubfonts", json!({"insert": "blank", "name": "Dingbats"}));
        run(&mut s, "cid.editSubfonts", json!({"private": {"subfont": "Dingbats", "values": {"postscriptBlueFuzz": 0}}}));
        s
    };
    let expected = state(&make());
    for (file, params) in [("Cid3.glyphs", json!({"version": 3})), ("Cid.ufo", json!({}))] {
        let mut s = make();
        let path = dir.join(file);
        let mut p = params.clone();
        p["path"] = json!(path);
        run(&mut s, "file.saveAs", p);
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        let got = state(&back);
        assert_eq!(got.ros, expected.ros, "{file}");
        assert_eq!(got.cids, expected.cids, "{file}");
        let subfonts = |c: &CidFont| c.subfonts.iter().map(|f| (f.name.clone(), f.glyphs.clone())).collect::<Vec<_>>();
        assert_eq!(subfonts(&got), subfonts(&expected), "{file}");
        assert!(got.subfonts[1].private().and_then(|d| d.get("postscriptBlueFuzz")).is_some(), "{file}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
