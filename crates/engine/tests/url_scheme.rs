//! The URL scheme (`url.open`): fontcraft://show/ URLs open files and list the glyph layers to
//! show; every other URL is refused and changes nothing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-url-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `path` percent-encoded for a URL query value.
fn enc(path: &Path) -> String {
    let mut out = String::new();
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A font with A–Z and a–z on one master, a backup layer on A and G exported as uni01F4, saved
/// at `dir/file`. Returns the path, the master id and the backup layer's id.
fn write_font(dir: &Path, file: &str) -> (PathBuf, String, String) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({ "familyName": "Url Test" }));
    let master = s.font().unwrap().masters[0].id.clone();
    let backup = run(&mut s, "layers.addBackupLayer", json!({ "glyphs": ["A"], "name": "Backup" }));
    let backup = backup["layer"].as_str().unwrap().to_string();
    run(&mut s, "glyph.setProductionName", json!({ "glyphs": ["G"], "productionName": "uni01F4" }));
    let path = dir.join(file);
    run(&mut s, "file.saveAs", json!({ "path": path }));
    (path, master, backup)
}

/// The `[glyph, layer]` pairs of each tab in a url.open reply.
fn layers(reply: &Value) -> Vec<Vec<(String, String)>> {
    let tabs = reply["tabs"].as_array().unwrap();
    tabs.iter()
        .map(|t| {
            let glyphs = t["glyphs"].as_array().unwrap();
            glyphs.iter().map(|g| (g["glyph"].as_str().unwrap().to_string(), g["layer"].as_str().unwrap().to_string())).collect()
        })
        .collect()
}

fn pair(glyph: &str, layer: &str) -> (String, String) {
    (glyph.to_string(), layer.to_string())
}

#[test]
fn url_show_example_opens_two_files_with_their_layers() {
    // The reference's own example: two files, repeated layers on the first glyph, a production
    // name in the second file.
    let dir = temp("example");
    let (one, master, backup) = write_font(&dir, "One.fontcraft");
    let (two, master2, _) = write_font(&dir, "Two.fontcraft");
    let url = format!(
        "fontcraft://show/?path={}&glyph=A&layer={backup}&layer={master}&glyph=B&path={}&glyph=Gbreve&production=uni01F4",
        enc(&one),
        enc(&two)
    );
    let mut s = Session::new();
    let reply = run(&mut s, "url.open", json!({ "url": url }));
    assert_eq!(s.documents().len(), 2);
    let tabs = layers(&reply);
    assert_eq!(tabs[0], [pair("A", &backup), pair("A", &master), pair("B", &master)]);
    // Gbreve is not in the font: skipped with a warning; uni01F4 is G's production name.
    assert_eq!(tabs[1], [pair("G", &master2)]);
    assert_eq!(reply["tabs"][0]["opened"], true);
    let warnings = reply["warnings"].to_string();
    assert!(warnings.contains("Gbreve"), "{warnings}");
    // The last file's document is the active one.
    assert_eq!(s.doc().unwrap().path.as_deref(), Some(two.as_path()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn url_show_production_names_and_default_layer() {
    let dir = temp("production");
    let (path, master, _) = write_font(&dir, "Font.fontcraft");
    let mut s = Session::new();
    // A glyph without a production name of its own is found by its name; G by uni01F4.
    let url = format!("fontcraft://show?path={}&production=uni01F4&production=B&glyph=c", enc(&path));
    let reply = run(&mut s, "url.open", json!({ "url": url }));
    assert_eq!(layers(&reply)[0], [pair("G", &master), pair("B", &master), pair("c", &master)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn url_show_does_not_reopen_an_open_file() {
    let dir = temp("open");
    let (path, master, _) = write_font(&dir, "Open.fontcraft");
    let mut s = Session::new();
    run(&mut s, "file.open", json!({ "path": path }));
    run(&mut s, "glyph.newGlyph", json!({ "name": "A.alt" }));
    run(&mut s, "file.new", json!({}));
    let url = format!("fontcraft://show/?path={}&glyph=A.alt", enc(&path));
    let reply = run(&mut s, "url.open", json!({ "url": url }));
    // No second copy; the unsaved glyph is still there and becomes the active document's.
    assert_eq!(s.documents().len(), 2);
    assert_eq!(reply["tabs"][0]["opened"], false);
    assert_eq!(layers(&reply)[0], [pair("A.alt", &master)]);
    assert!(s.font().unwrap().glyph("A.alt").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn url_show_percent_decodes_values() {
    let root = temp("decode");
    let dir = root.join("Schrift ü+1");
    std::fs::create_dir_all(&dir).unwrap();
    let (path, _, backup) = write_font(&dir, "My Font.fontcraft");
    let mut s = Session::new();
    // The scheme and action in capitals, a literal `+` kept as a plus sign, lower-case hex digits.
    let first = backup.bytes().next().unwrap();
    let layer = format!("%{first:02x}{}", backup.get(1..).unwrap());
    let url = format!("FontCraft://SHOW/?path={}&glyph=%41&layer={layer}", enc(&path).replace("%2B", "+"));
    let reply = run(&mut s, "url.open", json!({ "url": url }));
    assert_eq!(layers(&reply)[0], [pair("A", &backup)]);
    assert_eq!(s.doc().unwrap().path.as_deref(), Some(path.as_path()));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn url_show_skips_unknown_glyphs_and_layers_with_warnings() {
    let dir = temp("unknown");
    let (path, master, _) = write_font(&dir, "Font.fontcraft");
    let mut s = Session::new();
    let url = format!(
        "fontcraft://show/?glyph=A&path={}&layer=early&glyph=A&glyph=nosuch&layer=x&glyph=B&layer=nolayer&layer={master}&colour=red&path=relative/Font.fontcraft&glyph=A",
        enc(&path)
    );
    let reply = run(&mut s, "url.open", json!({ "url": url }));
    assert_eq!(s.documents().len(), 1);
    assert_eq!(layers(&reply), [vec![pair("A", &master), pair("B", &master)]]);
    let warnings: Vec<String> = reply["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect();
    for expected in ["before any `path`", "follows no glyph", "nosuch", "`layer=x` follows no glyph", "nolayer", "colour", "not an absolute path"] {
        assert!(warnings.iter().any(|w| w.contains(expected)), "{expected} not in {warnings:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn url_show_refuses_other_urls_and_changes_nothing() {
    let dir = temp("refuse");
    let (path, _, _) = write_font(&dir, "Font.fontcraft");
    let p = enc(&path);
    let mut s = Session::new();
    let refused = [
        format!("fontcraft://run/?command=file.save&path={p}"),
        format!("fontcraft://show/extra?path={p}"),
        format!("fontcraft:show?path={p}"),
        format!("https://show/?path={p}"),
        format!("fontcraft://show/?path={p}#top"),
        format!("fontcraft://show/?path={p}&glyph=%4"),
        format!("fontcraft://show/?path={p}&glyph=%ZZ"),
        format!("fontcraft://show/?path={p}&glyph=%FF"),
        format!("fontcraft://show/?path={p}&glyph=A%00"),
        format!("fontcraft://show/?path={p}&glyph=A B"),
        format!("fontcraft://show/?path={p}&glyph={}", "A".repeat(64 * 1024)),
        format!("fontcraft://show/?{}", "path=/x&".repeat(65)),
        format!("fontcraft://show/?path={p}&glyph=A{}", "&layer=x".repeat(65)),
        "fontcraft://show/".to_string(),
        "fontcraft://show/?path=/no/such/dir/Font.fontcraft".to_string(),
    ];
    for url in refused {
        let shown: String = url.chars().take(80).collect();
        assert!(s.execute("url.open", &json!({ "url": url })).is_err(), "{shown} was accepted");
        assert!(s.documents().is_empty(), "{shown} opened a document");
    }
    assert!(s.journal().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn url_open_takes_the_request_as_files() {
    let dir = temp("files");
    let (path, master, backup) = write_font(&dir, "Font.fontcraft");
    let mut s = Session::new();
    let reply = run(
        &mut s,
        "url.open",
        json!({ "files": [{ "path": path, "glyphs": [{ "glyph": "A", "layers": [backup, master] }, { "production": "uni01F4" }] }] }),
    );
    assert_eq!(layers(&reply)[0], [pair("A", &backup), pair("A", &master), pair("G", &master)]);
    let both = json!({ "files": [{ "path": path, "glyphs": [{ "glyph": "A", "production": "A" }] }] });
    assert!(s.execute("url.open", &both).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
