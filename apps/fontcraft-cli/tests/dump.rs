//! `fontcraft-cli dump`, run as a user runs it (M12.76 options).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::process::Command;

use fontcraft_engine::Session;
use serde_json::json;

fn cli(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_fontcraft-cli")).args(args).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// A TrueType font with `A` at U+0041, a UnicodeData file and a GOADB, in a fresh folder.
fn files() -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("fontcraft-cli-dump-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    // A new font already holds `A`.
    s.execute("glyph.setUnicodes", &json!({ "glyphs": ["A"], "unicodes": ["0041"] })).unwrap();
    let ttf = dir.join("Font.ttf");
    s.execute("file.exportStatic", &json!({ "destination": ttf, "timestamp": 0 })).unwrap();
    let data = dir.join("UnicodeData.txt");
    std::fs::write(&data, "0041;LATIN CAPITAL LETTER A;Lu;0;L;;;;;N;;;;0061;\n").unwrap();
    let goadb = dir.join("GlyphOrderAndAliasDB");
    std::fs::write(&goadb, "A A.src\n").unwrap();
    (dir, ttf, data, goadb)
}

#[test]
fn dump_writes_unicode_names_maps_glyph_names_and_checks_the_glyphs_options() {
    let (dir, ttf, data, goadb) = files();
    let (ttf, data, goadb) = (ttf.to_str().unwrap(), data.to_str().unwrap(), goadb.to_str().unwrap());

    let (code, stdout, _) = cli(&["dump", "--help"]);
    assert_eq!(code, 0);
    for option in ["--unicode-data", "--allow-virtual-gids", "--fd", "--exclude-fd", "--flatten-seac", "--names", "--goadb", "avar=designspace"] {
        assert!(stdout.contains(option), "{option} is in the help: {stdout}");
    }

    let (code, named, stderr) = cli(&["dump", ttf, "-t", "cmap", "--unicode-data", data]);
    assert_eq!(code, 0, "{stderr}");
    assert!(named.contains("name=\"A\"/><!-- LATIN CAPITAL LETTER A -->"), "{named}");
    let (code, plain, _) = cli(&["dump", ttf, "-t", "cmap"]);
    assert_eq!(code, 0);
    assert!(!plain.contains("<!--"), "no names are bundled: {plain}");
    let (code, virtual_gids, _) = cli(&["dump", ttf, "-t", "cmap", "--allow-virtual-gids"]);
    assert_eq!((code, virtual_gids), (0, plain), "--allow-virtual-gids changes nothing");

    let (code, glyphs, stderr) = cli(&["dump", ttf, "-t", "@glyphs", "--goadb", goadb]);
    assert_eq!(code, 0, "{stderr}");
    assert!(glyphs.contains("{A.src,U+0041}"), "{glyphs}");
    let (code, glyphs, _) = cli(&["dump", ttf, "-t", "@glyphs", "--goadb", goadb, "--names", "final", "--glyphs", "A"]);
    assert_eq!(code, 0);
    assert!(glyphs.contains("{A,U+0041}"), "{glyphs}");
    let (code, glyphs, _) = cli(&["dump", ttf, "-t", "@glyphs", "--glyph-detail", "long", "--flatten-seac"]);
    assert_eq!(code, 0, "a font without seac glyphs dumps as before");
    assert!(glyphs.contains("endchar}"), "{glyphs}");

    assert_eq!(cli(&["dump", ttf, "-t", "@glyphs", "--names", "source"]).0, 2, "--names source needs --goadb");
    assert_eq!(cli(&["dump", ttf, "-t", "cmap", "--flatten-seac"]).0, 2, "--flatten-seac applies to @glyphs");
    assert_eq!(cli(&["dump", ttf, "-t", "cmap", "--unicode-data", data, "--format", "json"]).0, 2, "the JSON form has no comments");
    assert_eq!(cli(&["dump", ttf, "-t", "@glyphs", "--fd", "0"]).0, 4, "a TrueType font has no font dicts");
    assert_eq!(cli(&["dump", ttf, "-t", "avar=designspace"]).0, 4, "a static font has no fvar table");
    let missing = dir.join("missing.txt");
    assert_eq!(cli(&["dump", ttf, "-t", "cmap", "--unicode-data", missing.to_str().unwrap()]).0, 3, "an unreadable file is exit status 3");
    let _ = std::fs::remove_dir_all(&dir);
}
