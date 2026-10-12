//! A WOFF2 file with transformed tables opens, is inspected and lists its table directory.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

#[test]
fn a_woff2_file_opens_view_only() {
    let dir = std::env::temp_dir().join(format!("fontcraft-woff2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    let ttf = dir.join("Untitled.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    let woff2 = dir.join("Untitled.woff2");
    let wrapped = fontcraft_decompile::container::woff2::encode(&std::fs::read(&ttf).unwrap(), &Default::default()).unwrap();
    std::fs::write(&woff2, wrapped).unwrap();

    let from_ttf = run(&mut s, "file.open", json!({ "path": ttf }));
    let opened = run(&mut s, "file.open", json!({ "path": woff2 }));
    assert_eq!((opened["format"].as_str(), opened["viewOnly"].as_bool()), (Some("binary"), Some(true)));
    assert_eq!(opened["glyphs"], from_ttf["glyphs"]);

    let info = run(&mut s, "font.inspect", json!({ "input": woff2, "sections": ["format", "names"] }));
    assert_eq!((info["format"]["kind"].as_str(), info["names"]["family"].as_str()), (Some("WOFF2"), Some("Untitled")));

    let tables = run(&mut s, "tables.list", json!({ "font": woff2, "sort": "tag" }));
    assert_eq!(tables["format"], "WOFF2");
    let rows = tables["faces"][0]["tables"].as_array().unwrap();
    let row = |t: &str| rows.iter().find(|r| r["tag"] == t).unwrap();
    assert!(row("glyf")["transformLength"].is_u64(), "glyf is stored transformed");
    assert_eq!(row("loca")["transformLength"], 0);
    assert!(row("cmap").get("transformLength").is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}
