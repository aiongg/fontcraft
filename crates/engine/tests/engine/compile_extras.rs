//! The compile extras of `font.compile` (M12.42): collections, curve modes, implied on-curve
//! points, the subset switch and the stub `DSIG`, through the command.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;

use fontcraft_engine::Session;
use serde_json::{Value, json};
use skrifa::FontRef;
use skrifa::MetadataProvider;
use skrifa::raw::TableProvider;
use skrifa::raw::types::Tag;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-extras-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn two_masters(dir: &std::path::Path) -> Session {
    let path = dir.join("TestSans.glyphs");
    std::fs::write(&path, crate::export::TWO_MASTERS).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    s
}

/// The offset of each table of member `index` of the collection `bytes`, by tag.
fn member_offsets(bytes: &[u8], index: usize) -> Vec<(String, u32)> {
    let u32_at = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    let font = u32_at(12 + 4 * index) as usize;
    let tables = u16::from_be_bytes(bytes[font + 4..font + 6].try_into().unwrap()) as usize;
    (0..tables)
        .map(|i| {
            let at = font + 12 + 16 * i;
            (String::from_utf8_lossy(&bytes[at..at + 4]).into_owned(), u32_at(at + 8))
        })
        .collect()
}

#[test]
fn a_collection_holds_every_font_built_and_stores_identical_tables_once() {
    let dir = temp("collection");
    let mut s = two_masters(&dir);
    let file = dir.join("TestSans.ttc");
    let out = run(&mut s, "font.compile", json!({"destination": file, "timestamp": 0}));
    let fonts = out["fonts"].as_array().unwrap();
    assert!(fonts.len() >= 2, "{out}");
    assert!(fonts.iter().all(|f| f["files"].as_array().unwrap().is_empty()), "the members are not written on their own");
    assert_eq!(out["collections"][0]["fonts"], fonts.len());
    assert_eq!(out["collections"][0]["format"], "ttf");

    let bytes = std::fs::read(&file).unwrap();
    assert_eq!(&bytes[..4], b"ttcf");
    let names: Vec<String> = (0..fonts.len() as u32)
        .map(|i| FontRef::from_index(&bytes, i).unwrap().localized_strings(skrifa::string::StringId::POSTSCRIPT_NAME).next().unwrap().to_string())
        .collect();
    assert!(names.contains(&"TestSans-Regular".to_string()), "{names:?}");
    // Every member has the same glyphs and code points, so one cmap serves them all.
    let cmap = |i| member_offsets(&bytes, i).into_iter().find(|(t, _)| t == "cmap").unwrap().1;
    assert_eq!(cmap(0), cmap(1));
    assert!(member_offsets(&bytes, 0).iter().find(|(t, _)| t == "name") != member_offsets(&bytes, 1).iter().find(|(t, _)| t == "name"));

    // In a folder the collection is named by the document; nothing else is written.
    let folder = dir.join("out");
    let out = run(&mut s, "font.compile", json!({"destination": folder, "collection": true, "formats": ["ttf", "otf"], "timestamp": 0}));
    let mut written: Vec<String> = std::fs::read_dir(&folder).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    written.sort();
    assert_eq!(written, ["TestSans.otc", "TestSans.ttc"], "{out}");
    assert_eq!(&std::fs::read(folder.join("TestSans.otc")).unwrap()[..4], b"ttcf");
}

#[test]
fn a_collection_request_that_cannot_be_written_writes_nothing() {
    let dir = temp("collection-refused");
    let mut s = two_masters(&dir);
    let otc = dir.join("x.otc");
    let e = s.execute("font.compile", &json!({"destination": otc, "formats": ["ttf"]})).unwrap_err().to_string();
    assert!(e.contains(".otc"), "{e}");
    let e = s.execute("font.compile", &json!({"collection": true, "containers": ["woff2"], "destination": dir.join("out")})).unwrap_err().to_string();
    assert!(e.contains("plain"), "{e}");
    assert!(!otc.exists() && !dir.join("out").exists());
}

#[test]
fn compile_takes_the_curve_mode_implied_points_subset_and_dsig_switches() {
    let mut s = Session::with_document();
    let square = json!([{"x": 0, "y": 0}, {"x": 500, "y": 0}, {"x": 500, "y": 500}, {"x": 0, "y": 500}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": square}));
    let dir = temp("switches");
    let file = dir.join("a.ttf");
    let p = json!({
        "destination": file, "timestamp": 0, "stubDsig": true, "subset": false,
        "outlines": {"curves": "keep-quad", "dropImpliedOnCurves": false},
    });
    run(&mut s, "font.compile", p);
    let bytes = std::fs::read(&file).unwrap();
    let font = FontRef::new(&bytes).unwrap();
    assert_eq!(font.table_data(Tag::new(b"DSIG")).unwrap().as_bytes(), [0, 0, 0, 1, 0, 0, 0, 1]);
    assert!(font.glyf().is_ok());

    for (curves, wanted) in [("mixed", "glyf"), ("keep-cubic", "glyf"), ("quad", "keep-quad")] {
        let e = s.execute("font.compile", &json!({"outlines": {"curves": curves}})).unwrap_err().to_string();
        assert!(e.contains(wanted), "{curves}: {e}");
    }
    let e = s.execute("font.compile", &json!({"outlines": {"dropImpliedOnCurves": "yes"}})).unwrap_err().to_string();
    assert!(e.contains("dropImpliedOnCurves"), "{e}");
}

#[test]
fn tables_kept_on_open_pass_through_unchanged_until_the_glyph_order_changes() {
    let dir = temp("pass-through");
    let mut s = Session::with_document();
    let square = json!([{"x": 0, "y": 0}, {"x": 500, "y": 0}, {"x": 500, "y": 500}, {"x": 0, "y": 500}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": square}));
    let built = dir.join("built.ttf");
    run(&mut s, "font.compile", json!({"destination": built, "timestamp": 0}));
    let glyphs = FontRef::new(&std::fs::read(&built).unwrap()).unwrap().maxp().unwrap().num_glyphs();

    // The legacy tables are added as raw bytes; nothing in the build reads their contents.
    let mut sfnt = fontcraft_tables::sfnt::read(&std::fs::read(&built).unwrap(), 0).unwrap();
    let mut ltsh = vec![0, 0];
    ltsh.extend(glyphs.to_be_bytes());
    ltsh.extend(std::iter::repeat_n(1u8, usize::from(glyphs)));
    let kept: [(&[u8; 4], Vec<u8>); 3] =
        [(b"LTSH", ltsh), (b"hdmx", vec![0, 0, 0, 0, 0, 0, 0, 4]), (b"trak", vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])];
    for (tag, data) in &kept {
        sfnt.insert(**tag, data.clone());
    }
    let legacy = dir.join("legacy.ttf");
    std::fs::write(&legacy, fontcraft_tables::sfnt::write(&sfnt, &Default::default()).unwrap()).unwrap();

    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": legacy }));
    let copied = dir.join("copied.ttf");
    run(&mut s, "font.compile", json!({"destination": copied, "timestamp": 0, "passThroughTables": true}));
    let bytes = std::fs::read(&copied).unwrap();
    let font = FontRef::new(&bytes).unwrap();
    for (tag, data) in &kept {
        assert_eq!(font.table_data(Tag::new(tag)).map(|d| d.as_bytes().to_vec()).as_ref(), Some(data), "{}", String::from_utf8_lossy(*tag));
    }

    // Without the switch, and once the glyph order is no longer the binary's, nothing is copied.
    let plain = dir.join("plain.ttf");
    run(&mut s, "font.compile", json!({"destination": plain, "timestamp": 0}));
    assert!(FontRef::new(&std::fs::read(&plain).unwrap()).unwrap().table_data(Tag::new(b"LTSH")).is_none());
    run(&mut s, "glyph.newGlyph", json!({"name": "A.alt"}));
    let changed = dir.join("changed.ttf");
    let out = run(&mut s, "font.compile", json!({"destination": changed, "timestamp": 0, "passThroughTables": true}));
    assert!(out.to_string().contains("no longer the one they were made for"), "{out}");
    let bytes = std::fs::read(&changed).unwrap();
    let font = FontRef::new(&bytes).unwrap();
    assert!(kept.iter().all(|(tag, _)| font.table_data(Tag::new(tag)).is_none()));
}
