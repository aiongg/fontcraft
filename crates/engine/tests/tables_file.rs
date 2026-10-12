//! Table actions on compiled font files: `tables.extract`, `.add`, `.delete`, `.edit`,
//! `.checksum`, `.reorder`, `.show`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-tables-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An exported TrueType font in `dir`.
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
fn tables_extract_add_delete_and_edit_work_on_files() {
    let dir = temp("edit");
    let (mut s, ttf) = font(&dir);
    let original = std::fs::read(&ttf).unwrap();

    let r = run(&mut s, "tables.extract", json!({ "font": ttf, "tables": ["OS/2", "name", "GPOS"] }));
    assert_eq!(r["files"][0]["path"], json!(dir.join("Font.OS_2")), "non-alphanumerics become underscores");
    assert_eq!(std::fs::read(dir.join("Font.name")).unwrap(), table(&ttf, b"name").unwrap());
    assert_eq!(r["warnings"].as_array().unwrap().len(), 1, "a missing table is a warning");

    let zzzz = dir.join("zzzz.bin");
    std::fs::write(&zzzz, [1, 2, 3]).unwrap();
    let r = run(&mut s, "tables.add", json!({ "font": ttf, "tables": [format!("zzzz={}", zzzz.display())] }));
    let edited = PathBuf::from(r["output"].as_str().unwrap());
    assert_eq!(edited, dir.join("Font.edited.ttf"));
    assert_eq!(table(&edited, b"zzzz"), Some(vec![1, 2, 3]));
    assert_eq!(table(&edited, b"glyf"), table(&ttf, b"glyf"), "untouched tables keep their bytes");
    assert_eq!(std::fs::read(&ttf).unwrap(), original, "the input is never changed");
    assert!(run(&mut s, "tables.checksum", json!({ "font": edited }))["ok"].as_bool().unwrap());

    let r = run(&mut s, "tables.delete", json!({ "font": edited, "tables": "zzzz,GPOS", "inPlace": true }));
    assert_eq!((r["deleted"].clone(), r["warnings"].as_array().unwrap().len()), (json!(["zzzz"]), 1));
    assert_eq!(table(&edited, b"zzzz"), None);

    // Extract, then delete, then add, whatever order the parameters come in.
    let out = dir.join("Edited2.ttf");
    let saved = dir.join("saved.name");
    let r = run(
        &mut s,
        "tables.edit",
        json!({ "font": ttf, "add": [format!("name={}", zzzz.display())], "delete": ["name"], "extract": [format!("name={}", saved.display())], "output": out }),
    );
    assert_eq!(r["replaced"], json!([]), "name was deleted before it was added");
    assert_eq!(table(&out, b"name"), Some(vec![1, 2, 3]));
    assert_eq!(std::fs::read(&saved).unwrap(), table(&ttf, b"name").unwrap());

    // Copying between fonts.
    run(&mut s, "tables.add", json!({ "font": ttf, "tables": [{ "tag": "name", "from": { "font": out } }], "output": dir.join("C.ttf") }));
    assert_eq!(table(&dir.join("C.ttf"), b"name"), Some(vec![1, 2, 3]));
    assert!(
        s.execute("tables.add", &json!({ "font": ttf, "tables": [{ "tag": "GPOS", "from": { "font": ttf } }], "output": dir.join("D.ttf") }))
            .is_err()
    );
    assert!(!dir.join("D.ttf").exists(), "a failed command leaves no output");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tables_checksum_finds_and_fixes_bad_values() {
    let dir = temp("checksum");
    let (mut s, ttf) = font(&dir);
    let mut bytes = std::fs::read(&ttf).unwrap();
    bytes[12 + 4] ^= 0xFF; // the first directory record's checksum
    let bad = dir.join("Bad.ttf");
    std::fs::write(&bad, &bytes).unwrap();
    let r = run(&mut s, "tables.checksum", json!({ "font": bad }));
    assert_eq!(r["ok"], false);
    assert_eq!(r["faces"][0]["mismatches"].as_array().unwrap().len(), 1);
    let r = run(&mut s, "tables.checksum", json!({ "font": bad, "fix": true }));
    let fixed = PathBuf::from(r["output"].as_str().unwrap());
    assert_eq!(fixed, dir.join("Bad.fixed.ttf"));
    assert_eq!(run(&mut s, "tables.checksum", json!({ "font": fixed }))["ok"], true);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tables_reorder_and_show_read_the_font() {
    let dir = temp("show");
    let (mut s, ttf) = font(&dir);
    let r = run(&mut s, "tables.reorder", json!({ "font": ttf, "order": ["post", "name"], "output": dir.join("R.ttf") }));
    let order: Vec<&str> = r["order"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
    assert_eq!(&order[..2], ["post", "name"]);
    let listed = run(&mut s, "tables.list", json!({ "font": dir.join("R.ttf") }));
    let by_offset: Vec<&str> = listed["faces"][0]["tables"].as_array().unwrap().iter().map(|t| t["tag"].as_str().unwrap()).collect();
    assert_eq!(&by_offset[..2], ["post", "name"], "the file lays the tables out in that order");

    let shown = run(&mut s, "tables.show", json!({ "font": ttf, "tables": ["head", "cmap"] }));
    let head = &shown["tables"][0];
    assert_eq!((head["tag"].as_str(), head["decoded"].as_bool()), (Some("head"), Some(true)));
    assert!(head["tree"]["children"].as_array().unwrap().iter().any(|c| c["name"] == "unitsPerEm" && c["value"] == "1000"));
    let one = run(&mut s, "tables.show", json!({ "font": ttf, "path": "cmap.encodingRecords[0]", "depth": 1, "offsets": true }));
    let tree = &one["tables"][0]["tree"];
    assert_eq!(tree["name"], "0");
    assert!(tree["children"].as_array().unwrap().iter().any(|c| c["name"] == "subtable" && c["offset"].is_u64() && c["more"].as_u64() > Some(0)));
    assert!(s.execute("tables.show", &json!({ "font": ttf, "path": "cmap.nothing" })).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
