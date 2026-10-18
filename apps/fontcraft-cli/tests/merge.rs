//! `fontcraft-cli merge`, run as a user runs it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::process::Command;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn cli(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_fontcraft-cli")).args(args).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn merge_combines_fonts_and_attaches_maps_to_the_font_before_them() {
    let dir = std::env::temp_dir().join(format!("fontcraft-cli-merge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ttf = dir.join("Font.ttf");
    let mut s = Session::with_document();
    s.execute("file.exportStatic", &json!({ "destination": ttf, "timestamp": 0 })).unwrap();
    let ttf_s = ttf.to_str().unwrap();
    let out = dir.join("Merged.ttf");
    let out_s = out.to_str().unwrap();

    let (code, stdout, stderr) = cli(&["merge", ttf_s, ttf_s, "-o", out_s, "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let r: Value = serde_json::from_str(&stdout).unwrap();
    let count = r["glyphCount"].as_u64().unwrap();
    let names: Vec<&str> = r["glyphs"].as_array().unwrap().iter().map(|g| g["name"].as_str().unwrap()).collect();
    assert!(names.contains(&".notdef.1"), "the second copy is renamed: {names:?}");
    assert!(out.exists());

    // A map after the second FONT takes one glyph from it.
    let map = dir.join("one.map");
    std::fs::write(&map, ".notdef.copy .notdef\n").unwrap();
    let out2 = dir.join("Mapped.ttf");
    let (code, stdout, stderr) = cli(&["merge", ttf_s, ttf_s, "--map", map.to_str().unwrap(), "-o", out2.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let r: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(r["glyphCount"].as_u64().unwrap(), count / 2 + 1);

    let (code, _, _) = cli(&["merge", "--map", map.to_str().unwrap(), ttf_s, "-o", out_s]);
    assert_eq!(code, 2, "a per-font option before any FONT is a usage error");
    let (code, _, _) = cli(&["merge", ttf_s, ttf_s]);
    assert_eq!(code, 2, "the output is required");
    let (code, _, _) = cli(&["merge", ttf_s, dir.join("missing.ttf").to_str().unwrap(), "-o", out_s]);
    assert_eq!(code, 3, "an unreadable input is exit status 3");
    let (code, stdout, _) = cli(&["merge", "--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("--map"));
    let _ = std::fs::remove_dir_all(&dir);
}
