//! `tables.importTtx`: tables compiled from TTX dumps into a compiled font file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-import-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn font(dir: &Path) -> (Session, PathBuf) {
    let mut s = Session::with_document();
    let nodes = json!([{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": nodes}));
    let ttf = dir.join("Font.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    (s, ttf)
}

fn table(path: &Path, tag: &[u8; 4]) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).unwrap();
    fontcraft_tables::read(&bytes, 0).unwrap().get(*tag).map(<[u8]>::to_vec)
}

#[test]
fn tables_import_ttx_replaces_the_dumped_tables_and_keeps_the_rest() {
    let dir = temp("edit");
    let (mut s, ttf) = font(&dir);
    let original = std::fs::read(&ttf).unwrap();
    let dumped = run(&mut s, "font.dumpTables", json!({ "font": ttf, "tables": ["name", "OS/2"] }));
    let text = dumped["text"].as_str().unwrap();
    let edited_text = text.replace("<usWeightClass value=\"400\"/>", "<usWeightClass value=\"700\"/>");
    assert_ne!(edited_text, text, "the dump holds the weight class");
    let dump = dir.join("patch.ttx");
    std::fs::write(&dump, &edited_text).unwrap();

    let r = run(&mut s, "tables.importTtx", json!({ "font": ttf, "dumps": [dump] }));
    let out = PathBuf::from(r["output"].as_str().unwrap());
    assert_eq!(out, dir.join("Font.edited.ttf"));
    assert!(r["replaced"].as_array().unwrap().contains(&json!("OS/2")), "{r}");
    assert_eq!(r["added"], json!([]), "{r}");
    let os2 = table(&out, b"OS/2").unwrap();
    assert_eq!(u16::from_be_bytes([os2[4], os2[5]]), 700);
    assert_eq!(table(&out, b"glyf"), table(&ttf, b"glyf"), "tables the dump does not hold keep their bytes");
    assert_eq!(std::fs::read(&ttf).unwrap(), original, "the input is never changed");
    assert!(run(&mut s, "tables.checksum", json!({ "font": out }))["ok"].as_bool().unwrap());
}

#[test]
fn tables_import_ttx_refuses_a_bad_dump_and_writes_nothing() {
    let dir = temp("refuse");
    let (mut s, ttf) = font(&dir);
    let dump = dir.join("bad.ttx");
    std::fs::write(&dump, "<?xml version=\"1.0\"?>\n<ttFont><OS_2><version value=\"9\"/></OS_2></ttFont>\n").unwrap();
    assert!(s.execute("tables.importTtx", &json!({ "font": ttf, "dumps": [dump] })).is_err());
    assert!(!dir.join("Font.edited.ttf").exists(), "a failed import writes no font");
    std::fs::write(&dump, "not xml").unwrap();
    assert!(s.execute("tables.importTtx", &json!({ "font": ttf, "dumps": [dump] })).is_err());
}

#[test]
fn tables_import_ttx_refuses_split_dump_parts_outside_its_folder() {
    let dir = temp("split");
    let (mut s, ttf) = font(&dir);
    std::fs::create_dir_all(dir.join("dump")).unwrap();
    let os2 = run(&mut s, "font.dumpTables", json!({ "font": ttf, "tables": ["OS/2"] }))["text"].as_str().unwrap().to_string();
    std::fs::write(dir.join("os2.ttx"), &os2).unwrap();
    std::fs::write(dir.join("dump").join("os2.ttx"), &os2).unwrap();
    let main = dir.join("dump").join("Font.ttx");
    for src in ["../os2.ttx", dir.join("os2.ttx").to_str().unwrap()] {
        std::fs::write(&main, format!("<?xml version=\"1.0\"?>\n<ttFont><OS_2 src=\"{src}\"/></ttFont>\n")).unwrap();
        let err = s.execute("tables.importTtx", &json!({ "font": ttf, "dumps": [main] })).unwrap_err();
        assert!(err.to_string().contains("parts must be beside it"), "{src}: {err}");
    }
    std::fs::write(&main, "<?xml version=\"1.0\"?>\n<ttFont><OS_2 src=\"os2.ttx\"/></ttFont>\n").unwrap();
    let r = run(&mut s, "tables.importTtx", json!({ "font": ttf, "dumps": [main] }));
    assert_eq!(r["added"], json!([]), "a part beside the main file is read: {r}");
}
