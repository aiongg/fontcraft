//! `font.convert` on compiled font files.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

#[test]
fn font_convert_changes_the_outline_flavour_and_writes_nothing_on_failure() {
    let dir = std::env::temp_dir().join(format!("fontcraft-convert-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    let square = json!([{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]);
    let curve = json!([
        {"x": 100, "y": 0}, {"x": 300, "y": 0, "type": "offcurve"}, {"x": 400, "y": 150, "type": "offcurve"}, {"x": 400, "y": 350, "type": "curve"},
        {"x": 400, "y": 700}, {"x": 100, "y": 700}
    ]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": square}));
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": curve}));
    let ttf = dir.join("Font.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    let original = std::fs::read(&ttf).unwrap();

    // TrueType to CFF, beside the input with the new extension.
    let r = run(&mut s, "font.convert", json!({ "input": ttf, "to": "otf" }));
    let otf = PathBuf::from(r["results"][0]["output"].as_str().unwrap());
    assert_eq!(otf, dir.join("Font.otf"));
    assert_eq!(std::fs::read(&ttf).unwrap(), original, "the input is never changed");
    let font = fontcraft_tables::read(&std::fs::read(&otf).unwrap(), 0).unwrap();
    assert!(font.get(*b"CFF ").is_some() && font.get(*b"glyf").is_none());
    assert_eq!(r["results"][0]["glyphs"], json!(font.get(*b"maxp").map(|m| u16::from_be_bytes([m[4], m[5]])).unwrap()));

    // CFF back to TrueType in another folder; CFF to CFF2 wrapped in WOFF2.
    let out_dir = dir.join("out");
    let r = run(&mut s, "font.convert", json!({ "inputs": [otf], "to": "ttf", "outputDir": out_dir, "curves": {"maxError": 0.001} }));
    let back = PathBuf::from(r["results"][0]["output"].as_str().unwrap());
    assert_eq!(back, out_dir.join("Font.ttf"));
    let font = fontcraft_tables::read(&std::fs::read(&back).unwrap(), 0).unwrap();
    assert!(font.get(*b"glyf").is_some() && font.get(*b"CFF ").is_none());
    let r = run(&mut s, "font.convert", json!({ "input": otf, "to": "otf-cff2", "container": "woff2", "output": dir.join("Font.cff2.woff2") }));
    let woff2 = std::fs::read(r["results"][0]["output"].as_str().unwrap()).unwrap();
    assert_eq!(&woff2[..4], b"wOF2");
    let font = fontcraft_tables::read(&woff2, 0).unwrap();
    assert!(font.get(*b"CFF2").is_some());

    // A run with one bad input writes none of its outputs.
    let junk = dir.join("notes.txt");
    std::fs::write(&junk, "not a font").unwrap();
    let failed = s.execute("font.convert", &json!({ "inputs": [ttf, junk], "to": "cff", "outputDir": dir.join("none") }));
    assert!(failed.is_err());
    assert!(!dir.join("none").exists(), "a failed conversion writes nothing");
    // Beside the inputs: the first output, already staged when the second input fails, is removed.
    let listing = || std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect::<std::collections::BTreeSet<_>>();
    let before = listing();
    assert!(s.execute("font.convert", &json!({ "inputs": [ttf, junk], "to": "cff" })).is_err());
    assert_eq!(listing(), before, "no output and no temporary file is left");
    assert!(s.execute("font.convert", &json!({ "input": ttf, "to": "designspace" })).is_err(), "sources are written by file.saveAs");
    assert!(s.execute("font.convert", &json!({ "input": ttf, "to": "otf", "fdSelect": [0] })).is_err(), "options not built yet are refused");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn font_convert_reads_ttx_text_and_bare_cff_tables() {
    let dir = std::env::temp_dir().join(format!("fontcraft-convert-programs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    let square = json!([{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": square}));
    let ttf = dir.join("Font.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    let cff_of = |path: PathBuf| fontcraft_tables::read(&std::fs::read(path).unwrap(), 0).unwrap().get(*b"CFF ").unwrap().to_vec();

    // A TTX dump converts as the font it was dumped from.
    let ttx = dir.join("Font.ttx");
    run(&mut s, "font.dumpTables", json!({ "font": ttf, "output": ttx }));
    run(&mut s, "font.convert", json!({ "input": ttf, "to": "otf", "output": dir.join("direct.otf") }));
    run(&mut s, "font.convert", json!({ "input": ttx, "to": "otf", "output": dir.join("via-ttx.otf") }));
    assert_eq!(cff_of(dir.join("via-ttx.otf")), cff_of(dir.join("direct.otf")));

    // A bare CFF table becomes a whole font again, with TrueType or CFF outlines.
    let bare = dir.join("Font.cff");
    run(&mut s, "font.convert", json!({ "input": ttf, "to": "cff", "output": bare }));
    let r = run(&mut s, "font.convert", json!({ "input": bare, "to": "ttf", "output": dir.join("from-cff.ttf") }));
    let font = fontcraft_tables::read(&std::fs::read(dir.join("from-cff.ttf")).unwrap(), 0).unwrap();
    assert!(font.get(*b"glyf").is_some() && font.get(*b"cmap").is_some() && font.get(*b"OS/2").is_some());
    let direct = fontcraft_tables::read(&std::fs::read(dir.join("direct.otf")).unwrap(), 0).unwrap();
    assert_eq!(r["results"][0]["glyphs"], json!(direct.get(*b"maxp").map(|m| u16::from_be_bytes([m[4], m[5]])).unwrap()));
    // Through OpenType (subroutinised) and back to a bare table: the charstrings are kept.
    run(&mut s, "font.convert", json!({ "input": bare, "to": "otf", "output": dir.join("from-cff.otf") }));
    assert!(cff_of(dir.join("from-cff.otf")).len() > 4);
    let back = dir.join("back.cff");
    run(&mut s, "font.convert", json!({ "input": dir.join("from-cff.otf"), "to": "cff", "cff": {"subroutinize": false}, "output": back }));
    run(&mut s, "font.convert", json!({ "input": bare, "to": "cff", "output": dir.join("again.cff") }));
    assert_eq!(std::fs::read(&back).unwrap(), std::fs::read(dir.join("again.cff")).unwrap());
    assert!(s.execute("font.convert", &json!({ "input": bare, "to": "otf", "face": 1, "output": dir.join("x.otf") })).is_err(), "one font: no face");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn font_convert_writes_ufos_glyph_subsets_and_goadb_names() {
    let dir = std::env::temp_dir().join(format!("fontcraft-convert-steps-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    let square = json!([{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": square}));
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": square}));
    let ttf = dir.join("Font.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    let names_of = |path: &str| fontcraft_tables::reorder::glyph_names(&fontcraft_tables::read(&std::fs::read(path).unwrap(), 0).unwrap()).unwrap();

    // To UFO: a folder beside the input, replaced on a second run; a file of that name is not.
    let r = run(&mut s, "font.convert", json!({ "input": ttf, "to": "ufo" }));
    let ufo = PathBuf::from(r["results"][0]["output"].as_str().unwrap());
    assert_eq!(ufo, dir.join("Font.ufo"));
    assert!(ufo.join("metainfo.plist").is_file() && ufo.join("fontinfo.plist").is_file());
    let glifs: Vec<String> = std::fs::read_dir(ufo.join("glyphs")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(glifs.iter().any(|g| g.starts_with("A")) && glifs.iter().any(|g| g.starts_with("B")), "{glifs:?}");
    run(&mut s, "font.convert", json!({ "input": ttf, "to": "ufo", "excludeGlyphs": "B" }));
    let glifs: Vec<String> = std::fs::read_dir(ufo.join("glyphs")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(!glifs.iter().any(|g| g.starts_with("B")), "the second run replaced the UFO: {glifs:?}");
    let err = s.execute("font.convert", &json!({ "input": ttf, "to": "ufo", "output": dir.join("Stamped.ufo"), "timestamp": "now" })).unwrap_err();
    assert!(err.to_string().contains("timestamp"), "a key without effect on a UFO is refused: {err}");
    assert!(!dir.join("Stamped.ufo").exists());
    std::fs::write(dir.join("Taken.ufo"), "a file").unwrap();
    assert!(s.execute("font.convert", &json!({ "input": ttf, "to": "ufo", "output": dir.join("Taken.ufo") })).is_err());

    // A glyph subset keeps .notdef and the glyphs named; an unknown name is refused.
    let r = run(&mut s, "font.convert", json!({ "input": ttf, "to": "otf", "glyphs": ["A"] }));
    let names = names_of(r["results"][0]["output"].as_str().unwrap());
    assert!(names.contains(&"A".to_string()) && !names.contains(&"B".to_string()), "{names:?}");
    let err = s.execute("font.convert", &json!({ "input": ttf, "to": "otf", "glyphs": ["A", "nope"] })).unwrap_err();
    assert!(err.to_string().contains("nope"), "{err}");

    // A GOADB renames by default; --no-rename with --subset-to-goadb only subsets.
    let goadb = dir.join("GlyphOrderAndAliasDB");
    std::fs::write(&goadb, "Ay A\n").unwrap();
    let r = run(&mut s, "font.convert", json!({ "input": ttf, "to": "ttf", "output": dir.join("Renamed.ttf"), "goadb": goadb }));
    let names = names_of(r["results"][0]["output"].as_str().unwrap());
    assert!(names.contains(&"Ay".to_string()) && names.contains(&"B".to_string()) && !names.contains(&"A".to_string()), "{names:?}");
    let r = run(
        &mut s,
        "font.convert",
        json!({ "input": ttf, "to": "ttf", "output": dir.join("Kept.ttf"), "goadb": goadb, "goadbRename": false, "goadbSubset": true }),
    );
    let names = names_of(r["results"][0]["output"].as_str().unwrap());
    assert!(names.contains(&"A".to_string()) && !names.contains(&"B".to_string()), "{names:?}");
    assert!(s.execute("font.convert", &json!({ "input": ttf, "to": "otf", "goadbOrder": true })).is_err(), "GOADB keys need a GOADB");

    // A location needs a variable font.
    assert!(s.execute("font.convert", &json!({ "input": ttf, "to": "otf", "location": {"wght": 400} })).is_err());

    // Overlap removal merges CFF outlines converted to CFF2, and to TrueType.
    let overlapping = json!([{"x": 300, "y": 300}, {"x": 700, "y": 300}, {"x": 700, "y": 900}, {"x": 300, "y": 900}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": overlapping}));
    let otf = dir.join("Overlapping.otf");
    run(&mut s, "file.exportStatic", json!({ "destination": otf, "outlines": "cff", "removeOverlap": false, "timestamp": 0 }));
    assert_eq!(contours(&std::fs::read(&otf).unwrap(), 'A'), 2);
    let r = run(&mut s, "font.convert", json!({ "input": otf, "to": "otf-cff2", "output": dir.join("Merged.otf"), "removeOverlaps": true }));
    assert_eq!(contours(&std::fs::read(r["results"][0]["output"].as_str().unwrap()).unwrap(), 'A'), 1);
    let r = run(&mut s, "font.convert", json!({ "input": otf, "to": "ttf", "output": dir.join("Merged.ttf"), "removeOverlaps": true }));
    assert_eq!(contours(&std::fs::read(r["results"][0]["output"].as_str().unwrap()).unwrap(), 'A'), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// How many contours glyph `name` of a compiled font has.
fn contours(bytes: &[u8], name: char) -> usize {
    struct Count(usize);
    impl skrifa::outline::OutlinePen for Count {
        fn move_to(&mut self, _: f32, _: f32) {
            self.0 += 1;
        }
        fn line_to(&mut self, _: f32, _: f32) {}
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn close(&mut self) {}
    }
    use skrifa::MetadataProvider as _;
    let font = skrifa::FontRef::new(bytes).unwrap();
    let glyph = font.outline_glyphs().get(font.charmap().map(name).unwrap()).unwrap();
    let mut pen = Count(0);
    let settings = skrifa::outline::DrawSettings::unhinted(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default());
    glyph.draw(settings, &mut pen).unwrap();
    pen.0
}

#[test]
fn font_convert_makes_cff_cid_keyed_and_selects_by_cid_and_font_dict() {
    let dir = std::env::temp_dir().join(format!("fontcraft-convert-cid-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    let square = json!([{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": square}));
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": square}));
    let ttf = dir.join("Font.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    let glyphs = fontcraft_tables::reorder::glyph_names(&fontcraft_tables::read(&std::fs::read(&ttf).unwrap(), 0).unwrap()).unwrap();
    let b = glyphs.iter().position(|n| n == "B").unwrap();

    // Name-keyed to CID-keyed: CID n is glyph n.
    let cid_otf = dir.join("Font-CID.otf");
    run(&mut s, "font.convert", json!({ "input": ttf, "to": "otf", "output": cid_otf, "cid": {"keyed": true} }));
    let font = fontcraft_tables::read(&std::fs::read(&cid_otf).unwrap(), 0).unwrap();
    let cids = fontcraft_tables::convert::glyph_cids(&font).unwrap().expect("a CID-keyed CFF table");
    assert_eq!(cids, (0..glyphs.len() as u16).collect::<Vec<_>>());

    // tx's /CID selector picks glyphs by CID; a Font DICT selection keeps the glyphs of dict 0.
    let r = run(&mut s, "font.convert", json!({ "input": cid_otf, "to": "otf", "output": dir.join("B.otf"), "glyphs": format!("/{b}") }));
    assert_eq!(r["results"][0]["glyphs"], json!(2));
    let r = run(&mut s, "font.convert", json!({ "input": cid_otf, "to": "otf", "output": dir.join("All.otf"), "fdSelect": [0] }));
    assert_eq!(r["results"][0]["glyphs"], json!(glyphs.len()));

    // Refused: a Font DICT the font lacks, a Font DICT or a CID in a name-keyed font, a CID the
    // font lacks, and CID keying of TrueType output.
    for bad in [
        json!({ "input": cid_otf, "to": "otf", "output": dir.join("x.otf"), "fdSelect": [1] }),
        json!({ "input": ttf, "to": "otf", "output": dir.join("x.otf"), "fdSelect": [0] }),
        json!({ "input": ttf, "to": "otf", "output": dir.join("x.otf"), "glyphs": "/1" }),
        json!({ "input": cid_otf, "to": "otf", "output": dir.join("x.otf"), "glyphs": "/9999" }),
        json!({ "input": cid_otf, "to": "otf", "output": dir.join("x.otf"), "glyphs": [3, "/1"] }),
        json!({ "input": ttf, "to": "ttf", "output": dir.join("x.ttf"), "cid": {"keyed": true} }),
        json!({ "input": ttf, "to": "otf", "output": dir.join("x.otf"), "cid": {"keyed": "Adobe-Identity"} }),
    ] {
        assert!(s.execute("font.convert", &bad).is_err(), "{bad}");
    }
    assert!(!dir.join("x.otf").exists() && !dir.join("x.ttf").exists());

    // CID ranges are merged before the glyphs are looked up: many copies of the widest range
    // cost the glyph count, and more items than a selector takes are refused before any work.
    let start = std::time::Instant::now();
    let many = vec!["/0-65535"; 60_000];
    let r = run(&mut s, "font.convert", json!({ "input": cid_otf, "to": "otf", "output": dir.join("Many.otf"), "glyphs": many }));
    assert_eq!(r["results"][0]["glyphs"], json!(glyphs.len()));
    let too_many = vec!["/0-65535"; 70_000];
    assert!(s.execute("font.convert", &json!({ "input": cid_otf, "to": "otf", "output": dir.join("x.otf"), "glyphs": too_many })).is_err());
    assert!(start.elapsed() < std::time::Duration::from_secs(5), "{:?}", start.elapsed());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn font_convert_puts_back_a_replaced_ufo_when_a_later_output_fails() {
    let dir = std::env::temp_dir().join(format!("fontcraft-convert-ufos-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    let square = json!([{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": square}));
    let (first, second) = (dir.join("First.ttf"), dir.join("Second.ttf"));
    run(&mut s, "file.exportStatic", json!({ "destination": first, "timestamp": 0 }));
    std::fs::copy(&first, &second).unwrap();
    let both = json!({ "inputs": [first, second], "to": "ufo" });
    run(&mut s, "font.convert", both.clone());
    std::fs::write(dir.join("First.ufo").join("marker.txt"), "the old UFO").unwrap();
    // A full folder where Second.ufo would be set aside makes its move fail after First.ufo
    // has been replaced.
    std::fs::create_dir_all(dir.join(format!(".Second.ufo.{}.old", std::process::id())).join("full")).unwrap();
    let err = s.execute("font.convert", &both).unwrap_err();
    assert!(err.to_string().contains("Second.ufo"), "{err}");
    assert!(dir.join("First.ufo").join("marker.txt").is_file(), "the replaced UFO is put back");
    assert!(dir.join("Second.ufo").join("metainfo.plist").is_file());
    let left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".First.ufo"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
