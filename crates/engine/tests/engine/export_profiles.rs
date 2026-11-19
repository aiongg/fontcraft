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
        json!({"folder": "Mine", "outlines": "cff", "containers": ["woff2", "plain"], "removeOverlap": true, "autohint": false, "cbdt": false})
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

/// A profile's `cbdt` setting reaches the compiler: the font it writes has `CBDT` and `CBLC`
/// (empty strikes here, since the iColor image file does not exist, with a warning), and a
/// profile without it writes neither.
#[test]
fn export_with_profiles_writes_cbdt_tables_when_the_profile_asks_for_them() {
    use skrifa::{FontRef, Tag};
    let dir = temp("cbdt");
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)}));
    s.edit("icolor layer", |font| {
        let mut a = font.glyph_mut("A")?;
        let mut l = fontcraft_model::Layer::new("m01", 600.0);
        l.id = "x1".into();
        a.layers.push(l);
        Ok(())
    })
    .unwrap();
    run(&mut s, "layer.convertToIColor", json!({"glyph": "A", "layer": "x1", "size": 64}));
    run(&mut s, "layer.setIColorImage", json!({"glyph": "A", "layer": "x1", "path": "A 64.png"}));

    let saved = run(&mut s, "file.saveExportProfile", json!({"name": "Bitmaps", "basedOn": "OpenType TT", "settings": {"cbdt": true}}));
    assert_eq!(saved["profile"]["params"]["cbdt"], true);
    assert!(fails(&mut s, "file.saveExportProfile", json!({"name": "Bitmaps", "settings": {"cbdt": "yes"}})).contains("cbdt"));
    let out = run(&mut s, "file.exportWithProfiles", json!({"profiles": ["Bitmaps", "OpenType TT"], "destination": dir, "timestamp": 0}));
    let has = |path: PathBuf, tag: &[u8; 4]| FontRef::new(&std::fs::read(path).unwrap()).unwrap().table_data(Tag::new(tag)).is_some();
    assert!(has(dir.join("Bitmaps/Untitled-Regular.ttf"), b"CBDT") && has(dir.join("Bitmaps/Untitled-Regular.ttf"), b"CBLC"), "{out}");
    assert!(!has(dir.join("OpenType TT/Untitled-Regular.ttf"), b"CBDT"), "{out}");
    assert!(out.to_string().contains("no strikes"), "the missing image is reported: {out}");
    std::fs::remove_dir_all(&dir).unwrap();
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

/// Adds to `name` a full-colour layer `color` holding the rectangle `x0, y0` to `x1, y1`, filled
/// red.
fn colour_glyph(s: &mut Session, name: &str, (x0, y0, x1, y1): (f64, f64, f64, f64)) {
    s.edit("Add Color Layer", |font| {
        let mut g = font.glyph_mut(name)?;
        let mut l = fontcraft_engine::model::Layer::new("m01", 600.0);
        l.id = "color".into();
        l.info.attrs.full_color = true;
        g.layers.push(l);
        Ok(())
    })
    .unwrap();
    run(s, "path.addPath", json!({"glyph": name, "layer": "color", "nodes": rect(x0, y0, x1, y1)}));
    run(s, "attributes.setFillColor", json!({"glyph": name, "layer": "color", "color": "#FF0000"}));
}

/// The PNG picture of glyph `name`'s colour layer at 200 pixels per em, decoded (straight RGBA).
fn colour_png(s: &mut Session, id: &str, name: &str) -> (fontcraft_engine::imagery::Rgba8, Value) {
    let out = run(s, id, json!({"glyphs": [name], "layer": "color", "format": "png", "pixelsPerEm": 200}));
    let data = base64::engine::general_purpose::STANDARD.decode(out["files"][0]["dataBase64"].as_str().unwrap()).unwrap();
    (fontcraft_engine::imagery::decode_rgba(&data, 1 << 24).unwrap(), out)
}

fn px(img: &fontcraft_engine::imagery::Rgba8, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * img.width + x) * 4) as usize;
    img.pixels[at..at + 4].try_into().unwrap()
}

/// The pixel box (left, top, right, bottom, inclusive) of the pixels `keep` accepts.
fn pixel_box(img: &fontcraft_engine::imagery::Rgba8, keep: impl Fn([u8; 4]) -> bool) -> (u32, u32, u32, u32) {
    let mut b = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..img.height {
        for x in 0..img.width {
            if keep(px(img, x, y)) {
                b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            }
        }
    }
    assert!(b.0 <= b.2, "no such pixel");
    b
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

#[test]
fn artwork_draws_a_shadow_below_and_offset_in_png_and_pdf_but_not_in_svg() {
    let mut s = Session::with_document();
    colour_glyph(&mut s, "A", (100.0, 100.0, 300.0, 300.0));
    let shadow = json!({"glyph": "A", "layer": "color", "attribute": "shadow", "blur": 0, "offset": [200, -200], "color": "#0000FFFF"});
    run(&mut s, "attributes.addAttribute", shadow);
    let (img, _) = colour_png(&mut s, "glyph.exportArtwork", "A");
    // 200 units at 200 pixels per em are 40 pixels: the shadow lies 40 right and 40 down.
    let (x0, y0, x1, y1) = pixel_box(&img, |p| p == RED);
    assert_eq!((x1 - x0 + 1, y1 - y0 + 1), (40, 40));
    let (cx, cy) = ((x0 + x1) / 2, (y0 + y1) / 2);
    assert_eq!(px(&img, cx, cy), RED);
    assert_eq!(px(&img, x1, y1), RED, "the path lies over its shadow");
    assert_eq!(px(&img, cx + 40, cy + 40), BLUE);
    assert_eq!(px(&img, cx + 40, cy)[3], 0, "no shadow beside the path");

    // Filter › Glyph as Image gives the same picture.
    let (alias, _) = colour_png(&mut s, "filter.glyphAsImage", "A");
    assert_eq!(alias, img);

    let out = run(&mut s, "glyph.exportArtwork", json!({"glyphs": ["A"], "layer": "color", "format": "pdf"}));
    let pdf = base64::engine::general_purpose::STANDARD.decode(out["files"][0]["dataBase64"].as_str().unwrap()).unwrap();
    let pdf = String::from_utf8_lossy(&pdf);
    assert!(pdf.contains("1 0 0 rg\n") && pdf.contains(" m\n"), "the fill is a red vector path");
    assert!(pdf.contains("/SMask") && pdf.contains("/Im0 Do"), "the shadow is an image");

    let out = run(&mut s, "glyph.exportArtwork", json!({"glyphs": ["A"], "layer": "color", "format": "svg"}));
    let svg = base64::engine::general_purpose::STANDARD.decode(out["files"][0]["dataBase64"].as_str().unwrap()).unwrap();
    let svg = String::from_utf8(svg).unwrap();
    assert_eq!(svg.matches("<path").count(), 1, "{svg}");
    assert!(!svg.contains("filter") && !svg.contains("image"), "SVG has no shadow: {svg}");
}

#[test]
fn artwork_draws_an_inner_shadow_inside_the_path() {
    let mut s = Session::with_document();
    colour_glyph(&mut s, "B", (100.0, 100.0, 300.0, 300.0));
    let inner = json!({"glyph": "B", "layer": "color", "attribute": "innerShadow", "blur": 0, "offset": [50, -50], "color": "#0000FFFF"});
    run(&mut s, "attributes.addAttribute", inner);
    let (img, _) = colour_png(&mut s, "glyph.exportArtwork", "B");
    let (x0, y0, x1, y1) = pixel_box(&img, |p| p[3] == 255);
    assert_eq!((x1 - x0 + 1, y1 - y0 + 1), (40, 40), "nothing is drawn outside the path");
    // The outside moved 10 pixels right and down shades the top and left edges inside.
    assert_eq!(px(&img, x0 + 3, y0 + 3), BLUE);
    assert_eq!(px(&img, x1 - 3, y0 + 3), BLUE);
    assert_eq!(px(&img, x1 - 3, y1 - 3), RED);
}

#[test]
fn artwork_tiles_a_pattern_image_over_the_fill_and_reports_one_it_cannot_read() {
    let dir = temp("pattern");
    // 400 by 1 image pixels, one per font unit: red then green.
    let pixels: Vec<u8> = (0..400).flat_map(|x| if x < 200 { [255, 0, 0, 255] } else { [0, 255, 0, 255] }).collect();
    let png = fontcraft_render::Image::from_rgba_premultiplied(400, 1, pixels).unwrap().png().unwrap();
    let file = dir.join("stripes.png");
    std::fs::write(&file, png).unwrap();

    let mut s = Session::with_document();
    colour_glyph(&mut s, "C", (0.0, 0.0, 400.0, 100.0));
    let pattern = json!({"glyph": "C", "layer": "color", "attribute": "patternImage", "image": file.to_string_lossy()});
    run(&mut s, "attributes.addAttribute", pattern);
    let (img, out) = colour_png(&mut s, "glyph.exportArtwork", "C");
    assert_eq!(out["warnings"], json!([]));
    let (x0, y0, _, y1) = pixel_box(&img, |p| p[3] == 255);
    let mid = (y0 + y1) / 2;
    // Units 0 to 200 are 0 to 40 pixels.
    assert_eq!(px(&img, x0 + 20, mid), RED);
    assert_eq!(px(&img, x0 + 60, mid), [0, 255, 0, 255]);

    let missing = json!({"glyph": "C", "layer": "color", "attribute": "patternImage", "image": dir.join("gone.png").to_string_lossy()});
    run(&mut s, "attributes.addAttribute", missing);
    let (img, out) = colour_png(&mut s, "glyph.exportArtwork", "C");
    assert_eq!(out["warnings"].as_array().unwrap().len(), 1, "{out}");
    assert!(out["warnings"][0].as_str().unwrap().contains("gone.png"));
    assert_eq!(px(&img, x0 + 60, mid), RED, "the path is drawn without its pattern");
    std::fs::remove_dir_all(&dir).unwrap();
}
