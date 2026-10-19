//! Export profiles (`file.saveExportProfile`, `file.removeExportProfile`,
//! `file.listExportProfiles`, `file.exportWithProfiles`) and glyph artwork
//! (`glyph.exportArtwork`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use base64::Engine as _;
use fontcraft_engine::Session;
use fontcraft_engine::settings::Reserved;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn fails(s: &mut Session, id: &str, p: Value) -> String {
    match s.execute(id, &p) {
        Ok(v) => panic!("{id} {p} should fail, got {v}"),
        Err(e) => e.to_string(),
    }
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-export-profiles-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Value {
    json!([{"x": x0, "y": y0}, {"x": x1, "y": y0}, {"x": x1, "y": y1}, {"x": x0, "y": y1}])
}

/// Every file below `dir`, relative, with `/` separators, sorted.
fn names(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        entries.flatten().flat_map(|e| if e.path().is_dir() { walk(&e.path()) } else { vec![e.path()] }).collect()
    }
    let mut out: Vec<String> = walk(dir).iter().map(|p| p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
    out.sort();
    out
}

/// Two open fonts with an outline in `A`: Untitled (active second) and Second.
fn two_fonts() -> (Session, Vec<String>) {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)}));
    run(&mut s, "file.new", json!({"familyName": "Second"}));
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(40.0, 0.0, 500.0, 700.0)}));
    let titles = s.documents().iter().map(|d| d.title.clone()).collect();
    (s, titles)
}

fn profile<'a>(list: &'a Value, name: &str) -> Option<&'a Value> {
    list["profiles"].as_array().unwrap().iter().find(|p| p["name"] == name)
}

#[test]
fn export_with_profiles_writes_two_fonts_through_two_profiles_into_subfolders() {
    let dir = temp("two");
    let (mut s, titles) = two_fonts();
    let out =
        run(&mut s, "file.exportWithProfiles", json!({"fonts": titles, "profiles": ["OpenType TT", "Web"], "destination": dir, "timestamp": 0}));
    assert_eq!(
        names(&dir),
        [
            "OpenType TT/Second-Regular.ttf",
            "OpenType TT/Untitled-Regular.ttf",
            "Web/Second-Regular.woff",
            "Web/Second-Regular.woff2",
            "Web/Untitled-Regular.woff",
            "Web/Untitled-Regular.woff2",
        ],
        "{out}"
    );
    assert_eq!(out["written"], 6);
    assert_eq!(out["profiles"][0]["profile"], "OpenType TT");
    assert_eq!(out["profiles"][1]["documents"].as_array().unwrap().len(), 2);

    // A user profile with an empty folder writes into the destination itself, CFF here.
    run(&mut s, "file.saveExportProfile", json!({"name": "Flat", "basedOn": "OpenType PS", "settings": {"folder": ""}}));
    let flat = temp("flat");
    run(&mut s, "file.exportWithProfiles", json!({"profiles": ["Flat"], "destination": flat, "timestamp": 0}));
    assert_eq!(names(&flat), ["Second-Regular.otf"], "only the active document without `fonts`");
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&flat).unwrap();
}

#[test]
fn export_with_profiles_refuses_and_writes_nothing() {
    let dir = temp("refused");
    let (mut s, titles) = two_fonts();
    std::fs::write(dir.join("keep.txt"), b"old").unwrap();
    let all = json!(titles);
    for (params, says) in [
        (json!({"profiles": ["Nope"], "destination": dir}), "Nope"),
        (json!({"profiles": [], "destination": dir}), "profiles"),
        (json!({"profiles": ["Web"]}), "destination"),
        (json!({"profiles": ["Web"], "destination": dir, "bySource": true}), "not both"),
        (json!({"profiles": ["Web"], "bySource": true}), "save it first"),
        (json!({"profiles": ["Web"], "fonts": ["Nowhere"], "destination": dir}), "Nowhere"),
        (json!({"profiles": ["Web"], "destination": dir.join("keep.txt")}), "is a file"),
        (json!({"profiles": ["Web"], "destination": dir, "installTemporarily": true}), "installTemporarily"),
    ] {
        let e = fails(&mut s, "file.exportWithProfiles", params.clone());
        assert!(e.contains(says), "{params}: {e}");
    }
    // Two profiles that would write the same files are refused before anything moves.
    run(&mut s, "file.saveExportProfile", json!({"name": "Twin", "basedOn": "OpenType TT", "settings": {"folder": "OpenType TT"}}));
    let e = fails(&mut s, "file.exportWithProfiles", json!({"fonts": all, "profiles": ["OpenType TT", "Twin"], "destination": dir}));
    assert!(e.contains("Twin") && e.contains("different folders"), "{e}");
    assert_eq!(names(&dir), ["keep.txt"], "no new files, no temporary files, no new folders");
    assert!(!dir.join("OpenType TT").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Whether the sfnt `font` has a table `tag`, read from its table directory.
fn has_table(font: &[u8], tag: &[u8; 4]) -> bool {
    let count = usize::from(u16::from_be_bytes([font[4], font[5]]));
    (0..count).any(|i| &font[12 + 16 * i..12 + 16 * i + 4] == tag)
}

/// A profile's autohint setting reaches the export: its TrueType fonts get the autohinter's
/// `fpgm`, and the same profile with autohinting off gets none.
#[test]
fn export_with_profiles_autohints_when_the_profile_says_so() {
    let dir = temp("autohint");
    let (mut s, _) = two_fonts();
    run(&mut s, "file.saveExportProfile", json!({"name": "Hinted", "basedOn": "OpenType TT", "settings": {"autohint": true}}));
    run(&mut s, "file.exportWithProfiles", json!({"profiles": ["Hinted", "OpenType TT"], "destination": dir, "timestamp": 0}));
    let hinted = std::fs::read(dir.join("Hinted/Second-Regular.ttf")).unwrap();
    let plain = std::fs::read(dir.join("OpenType TT/Second-Regular.ttf")).unwrap();
    assert!(has_table(&hinted, b"fpgm"), "the autohinted font has the autohinter's functions");
    assert!(!has_table(&plain, b"fpgm"), "a profile without autohint writes no hinting");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Every profile's files move into place as one step: a file that cannot be written in the last
/// profile leaves the files of the earlier profiles as they were.
#[test]
fn export_with_profiles_that_fails_in_a_later_profile_leaves_the_earlier_ones_untouched() {
    let dir = temp("partway");
    let (mut s, titles) = two_fonts();
    std::fs::create_dir_all(dir.join("OpenType TT")).unwrap();
    std::fs::write(dir.join("OpenType TT/Untitled-Regular.ttf"), b"old").unwrap();
    std::fs::create_dir_all(dir.join("Web/Untitled-Regular.woff2")).unwrap();
    let e =
        fails(&mut s, "file.exportWithProfiles", json!({"fonts": titles, "profiles": ["OpenType TT", "Web"], "destination": dir, "timestamp": 0}));
    assert!(e.contains("Untitled-Regular.woff2"), "{e}");
    assert_eq!(names(&dir), ["OpenType TT/Untitled-Regular.ttf"], "no new files, no temporary files");
    assert_eq!(std::fs::read(dir.join("OpenType TT/Untitled-Regular.ttf")).unwrap(), b"old");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn save_export_profile_keeps_built_in_profiles_read_only() {
    let mut s = Session::with_document();
    let list = run(&mut s, "file.listExportProfiles", json!({}));
    let names: Vec<&str> = list["profiles"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["OpenType PS", "OpenType TT", "Web"]);
    assert!(list["profiles"].as_array().unwrap().iter().all(|p| p["builtIn"] == true));

    for (id, params) in [
        ("file.saveExportProfile", json!({"name": "Web", "settings": {"containers": ["woff"]}})),
        ("file.removeExportProfile", json!({"name": "OpenType PS"})),
    ] {
        let e = fails(&mut s, id, params);
        assert!(e.contains("built-in"), "{id}: {e}");
    }

    // A copy starts from its base, writes into a folder named after itself, and keeps what an
    // update leaves out.
    let saved =
        run(&mut s, "file.saveExportProfile", json!({"name": "Mine", "basedOn": "OpenType PS", "settings": {"containers": ["woff2", "plain"]}}));
    assert_eq!(saved["created"], true);
    assert_eq!(
        saved["profile"]["params"],
        json!({"folder": "Mine", "outlines": "cff", "containers": ["woff2", "plain"], "removeOverlap": true, "autohint": false})
    );
    let updated = run(&mut s, "file.saveExportProfile", json!({"name": "Mine", "settings": {"removeOverlap": false}}));
    assert_eq!(updated["created"], false);
    assert_eq!(updated["profile"]["params"]["containers"], json!(["woff2", "plain"]));
    assert_eq!(updated["profile"]["params"]["removeOverlap"], false);

    // Bad settings change nothing.
    let before = s.settings().clone();
    for (settings, says) in [
        (json!({"folder": "../up"}), ".."),
        (json!({"folder": "/abs"}), "absolute"),
        (json!({"folder": "Web/con.d"}), "device name"),
        (json!({"folder": "Web. /x"}), "space or a dot"),
        (json!({"outlines": "type1"}), "outlines"),
        (json!({"containers": ["zip"]}), "containers"),
        (json!({"colour": true}), "colour"),
    ] {
        let e = fails(&mut s, "file.saveExportProfile", json!({"name": "Mine", "settings": settings}));
        assert!(e.contains(says), "{settings}: {e}");
    }
    assert_eq!(s.settings(), &before);

    // The settings file keeps the profile.
    let (reloaded, problems) = fontcraft_engine::settings::UserSettings::load(&s.settings().to_json());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(reloaded.reserved::<Value>(Reserved::ExportProfiles).unwrap().unwrap()[0]["name"], "Mine");

    run(&mut s, "file.removeExportProfile", json!({"name": "Mine"}));
    assert!(profile(&run(&mut s, "file.listExportProfiles", json!({})), "Mine").is_none());
}

#[test]
fn stored_profiles_that_cannot_be_read_are_reported_and_never_used() {
    let dir = temp("stored");
    let mut s = Session::with_document();
    let stored = json!([
        {"name": "Escape", "params": {"folder": "../../outside"}},
        {"name": "Web", "params": {}},
        {"name": "Good", "params": {"folder": "good", "containers": ["woff2"]}},
        "not an object",
    ]);
    s.settings_mut().set_reserved(Reserved::ExportProfiles, &stored).unwrap();
    let list = run(&mut s, "file.listExportProfiles", json!({}));
    assert!(profile(&list, "Escape").is_none());
    assert_eq!(profile(&list, "Good").unwrap()["params"]["containers"], json!(["woff2"]));
    assert_eq!(list["problems"].as_array().unwrap().len(), 3, "{list}");
    let e = fails(&mut s, "file.exportWithProfiles", json!({"profiles": ["Escape"], "destination": dir}));
    assert!(e.contains("Escape"), "{e}");
    assert!(names(&dir).is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn export_artwork_writes_svg_png_and_pdf_per_glyph() {
    let dir = temp("artwork");
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)}));
    let advance = s.font().unwrap().glyph("A").unwrap().layers[0].width;
    for format in ["svg", "pdf", "png"] {
        let out = run(&mut s, "glyph.exportArtwork", json!({"glyphs": ["A", "space"], "format": format, "directory": dir.join(format)}));
        assert_eq!(names(&dir.join(format)), [format!("A.{format}"), format!("space.{format}")], "{out}");
        assert_eq!(out["files"][0]["glyph"], "A");
    }
    let svg = std::fs::read_to_string(dir.join("svg/A.svg")).unwrap();
    assert!(svg.starts_with("<svg") && svg.contains("<path d=\"M"), "{svg}");
    let pdf = std::fs::read(dir.join("pdf/A.pdf")).unwrap();
    let pdf = String::from_utf8_lossy(&pdf);
    assert!(pdf.starts_with("%PDF-1.4") && pdf.contains(" m\n") && pdf.contains("f\n") && pdf.ends_with("%%EOF\n"), "{pdf}");
    let png = std::fs::read(dir.join("png/A.png")).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    // 256 pixels per em at 1000 units per em, over the advance or the outline, whichever is wider.
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    assert_eq!(width, (advance.max(560.0) * 0.256).ceil() as u32);

    // Without a directory the pictures come back in the result.
    let out = run(&mut s, "glyph.exportArtwork", json!({"glyphs": ["A"], "format": "svg"}));
    let data = base64::engine::general_purpose::STANDARD.decode(out["files"][0]["dataBase64"].as_str().unwrap()).unwrap();
    assert_eq!(String::from_utf8(data).unwrap(), svg);

    // Failures name the problem and write nothing.
    let deeper = dir.join("new/deeper");
    for (params, says) in [
        (json!({"glyphs": ["A"], "format": "gif", "directory": deeper}), "format"),
        (json!({"glyphs": ["A"], "format": "svg", "layer": "nope", "directory": deeper}), "nope"),
        (json!({"glyphs": ["A"], "format": "png", "pixelsPerEm": 0, "directory": deeper}), "pixelsPerEm"),
        (json!({"glyphs": ["A", "space"], "format": "png", "pixelsPerEm": 16384, "directory": deeper}), "glyph `A`"),
        (json!({"glyphs": ["A"], "format": "svg", "directory": dir.join("svg/A.svg")}), "is a file"),
    ] {
        let e = fails(&mut s, "glyph.exportArtwork", params.clone());
        assert!(e.contains(says), "{params}: {e}");
    }
    assert!(!dir.join("new").exists(), "no folder is left behind");
    std::fs::remove_dir_all(&dir).unwrap();
}
