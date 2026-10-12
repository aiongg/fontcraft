//! Placed images (M11.12): `glyph.addImage` with one file and with several matched by name,
//! `image.setTransform`, `image.lock`, `image.unlock`, `image.setCrop`,
//! `image.setCropToLayerBounds`, `image.resetCrop` and `image.getInfo`; a locked image refused by
//! every edit; paths rewritten when an unsaved document is first saved. Image files are built in
//! code: headers only, which is all placing an image reads.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use fontcraft_testkit::{snapshot, temp_dir};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn run_err(s: &mut Session, id: &str, p: Value) -> String {
    let before = snapshot(s);
    let e = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} {p} succeeded")).to_string();
    assert_eq!(snapshot(s), before, "{id} {p} failed but changed the session");
    e
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn info(s: &mut Session, glyph: &str) -> Value {
    run(s, "image.getInfo", json!({"glyph": glyph}))["image"].clone()
}

/// A folder of its own for one test.
fn folder(tag: &str) -> PathBuf {
    let dir = temp_dir("image").join(tag);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> String {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

fn chunk(b: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
    b.extend((data.len() as u32).to_be_bytes());
    b.extend(kind);
    b.extend(data);
    b.extend([0; 4]);
}

/// A PNG header of `width × height` pixels, with a `pHYs` chunk when `ppm` (pixels per metre)
/// is given.
fn png(width: u32, height: u32, ppm: Option<u32>) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(&mut b, b"IHDR", &ihdr);
    if let Some(ppm) = ppm {
        let mut phys = ppm.to_be_bytes().to_vec();
        phys.extend(ppm.to_be_bytes());
        phys.push(1);
        chunk(&mut b, b"pHYs", &phys);
    }
    chunk(&mut b, b"IEND", &[]);
    b
}

/// A JPEG header of `width × height` pixels at `dpi` dots per inch.
fn jpeg(width: u16, height: u16, dpi: u16) -> Vec<u8> {
    let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
    b.extend(b"JFIF\0\x01\x02\x01");
    b.extend(dpi.to_be_bytes());
    b.extend(dpi.to_be_bytes());
    b.extend([0, 0, 0xFF, 0xC0, 0, 11, 8]);
    b.extend(height.to_be_bytes());
    b.extend(width.to_be_bytes());
    b.extend([1, 1, 0x11, 0, 0xFF, 0xD9]);
    b
}

#[test]
fn add_image_places_one_file_at_the_origin_one_point_per_unit() {
    let dir = folder("single");
    let plain = write(&dir, "scan.png", &png(400, 300, None));
    // 5,669 pixels per metre: 144 pixels per inch, so half a unit per pixel.
    let fine = write(&dir, "fine.png", &png(400, 300, Some(5_669)));
    let photo = write(&dir, "photo.jpg", &jpeg(30, 20, 36));
    let mut s = Session::with_document();
    assert_eq!(info(&mut s, "A"), Value::Null, "no image yet");

    let r = run(&mut s, "glyph.addImage", json!({"files": [plain], "glyphs": ["A"]}));
    assert_eq!(r["unmatched"], json!([]));
    let placed = &r["placed"][0];
    assert_eq!((placed["glyph"].as_str(), placed["layer"].as_str(), placed["format"].as_str()), (Some("A"), Some("m01"), Some("png")));
    assert_eq!((placed["pixelWidth"].as_u64(), placed["scale"].clone(), placed["replaced"].as_bool()), (Some(400), json!([1.0, 1.0]), Some(false)));
    assert_eq!(undo_steps(&s), 1);
    // An unsaved document stores the absolute path.
    let i = info(&mut s, "A");
    assert_eq!(i["path"].as_str(), Some(plain.as_str()));
    assert_eq!((i["resolved"].as_bool(), i["x"].as_f64(), i["y"].as_f64()), (Some(true), Some(0.0), Some(0.0)));
    assert_eq!((i["width"].as_f64(), i["height"].as_f64(), i["rotation"].as_f64()), (Some(400.0), Some(300.0), Some(0.0)));
    assert_eq!((i["crop"].clone(), i["locked"].as_bool()), (Value::Null, Some(false)));

    // A second image replaces the first; the stated resolution sets the scale.
    let r = run(&mut s, "glyph.addImage", json!({"files": [fine], "glyphs": ["A", "B"]}));
    assert_eq!(r["placed"].as_array().unwrap().len(), 2);
    assert_eq!((r["placed"][0]["replaced"].as_bool(), r["placed"][1]["replaced"].as_bool()), (Some(true), Some(false)));
    let scale = r["placed"][0]["scale"][0].as_f64().unwrap();
    assert!((scale - 0.5).abs() < 1e-3, "{scale}");
    assert_eq!(undo_steps(&s), 2, "one step for every glyph of the call");
    let width = info(&mut s, "B")["width"].as_f64().unwrap();
    assert!((width - 200.0).abs() < 0.1, "{width}");

    // JPEG at 36 dpi: two units per pixel. A background image is a separate one.
    let r = run(&mut s, "glyph.addImage", json!({"files": photo, "glyphs": ["C"], "side": "background"}));
    assert_eq!(r["placed"][0]["scale"], json!([2.0, 2.0]));
    assert_eq!(info(&mut s, "C"), Value::Null);
    let back = run(&mut s, "image.getInfo", json!({"glyph": "C", "side": "background"}))["image"].clone();
    assert_eq!((back["format"].as_str(), back["width"].as_f64()), (Some("jpeg"), Some(60.0)));
}

#[test]
fn add_image_matches_several_files_to_glyph_names_exactly() {
    let dir = folder("batch");
    // Upper- and lower-case files in separate folders, as file systems that ignore case need.
    let upper = write(&dir, "upper/A.png", &png(10, 10, None));
    let lower = write(&dir, "lower/a.png", &png(20, 20, None));
    let alt = write(&dir, "upper/A.alt.png", &png(30, 30, None));
    let stray = write(&dir, "upper/Thorn-xx.png", &png(10, 10, None));
    let mut s = Session::with_document();
    run(&mut s, "glyph.newGlyph", json!({"name": "A.alt"}));
    let steps = undo_steps(&s);

    let r = run(&mut s, "glyph.addImage", json!({"files": [upper, lower, stray, alt]}));
    let placed: Vec<(&str, &str)> =
        r["placed"].as_array().unwrap().iter().map(|p| (p["glyph"].as_str().unwrap(), p["file"].as_str().unwrap())).collect();
    assert_eq!(placed, [("A", upper.as_str()), ("a", lower.as_str()), ("A.alt", alt.as_str())]);
    assert_eq!(r["unmatched"], json!([stray]));
    assert_eq!(undo_steps(&s), steps + 1, "one undo step for the whole call");
    assert_eq!(info(&mut s, "a")["pixelWidth"].as_u64(), Some(20), "a.png goes to a only");
    assert_eq!(info(&mut s, "A")["pixelWidth"].as_u64(), Some(10));

    // Nothing matched, two files for one glyph, several files with `glyphs`: refused.
    let e = run_err(&mut s, "glyph.addImage", json!({"files": [stray]}));
    assert!(e.contains("Thorn-xx.png"), "{e}");
    let jpg = write(&dir, "A.jpg", &jpeg(8, 8, 72));
    let e = run_err(&mut s, "glyph.addImage", json!({"files": [upper, jpg]}));
    assert!(e.contains("both name glyph `A`"), "{e}");
    run_err(&mut s, "glyph.addImage", json!({"files": [upper, lower], "glyphs": ["A"]}));
    run_err(&mut s, "glyph.addImage", json!({"files": []}));
}

#[test]
fn bad_files_refuse_the_whole_call() {
    let dir = folder("bad");
    let good = write(&dir, "B.png", &png(10, 10, None));
    let text = write(&dir, "C.png", b"not an image at all");
    let missing = dir.join("D.png").to_string_lossy().into_owned();
    let cut = write(&dir, "E.png", &png(10, 10, None)[..18]);
    let mut s = Session::with_document();
    for bad in [&text, &missing, &cut] {
        let e = run_err(&mut s, "glyph.addImage", json!({"files": [good, bad]}));
        assert!(e.contains(bad.rsplit('/').next().unwrap()), "{e}");
    }
    // A header claiming 100,000 × 100,000 pixels is placed: nothing is decoded here.
    let huge = write(&dir, "F.png", &png(100_000, 100_000, None));
    run(&mut s, "glyph.addImage", json!({"files": [huge]}));
    assert_eq!(info(&mut s, "F")["pixelWidth"].as_u64(), Some(100_000));
    // PDF and PSD are kept as references, with no pixel size.
    let pdf = write(&dir, "G.pdf", b"%PDF-1.4\n");
    run(&mut s, "glyph.addImage", json!({"files": [pdf]}));
    let i = info(&mut s, "G");
    assert_eq!((i["format"].as_str(), i["width"].clone()), (Some("pdf"), Value::Null));
    assert!(i["problem"].as_str().unwrap().contains("PDF"));
    run_err(&mut s, "image.setTransform", json!({"glyph": "G", "width": 100}));
    run(&mut s, "image.setTransform", json!({"glyph": "G", "x": 15}));
}

#[test]
fn set_transform_moves_sizes_and_rotates_the_image() {
    let dir = folder("transform");
    let file = write(&dir, "A.png", &png(400, 300, None));
    let mut s = Session::with_document();
    run(&mut s, "glyph.addImage", json!({"files": [file]}));
    let r = run(&mut s, "image.setTransform", json!({"glyph": "A", "x": 12.5, "y": -40, "width": 200, "height": 600, "rotation": -90}));
    let i = &r["image"];
    assert_eq!((i["x"].as_f64(), i["y"].as_f64(), i["rotation"].as_f64()), (Some(12.5), Some(-40.0), Some(270.0)));
    assert_eq!((i["width"].as_f64(), i["height"].as_f64(), i["scale"].clone()), (Some(200.0), Some(600.0), json!([0.5, 2.0])));
    run(&mut s, "image.setTransform", json!({"glyph": "A", "y": 0}));
    assert_eq!(info(&mut s, "A")["width"].as_f64(), Some(200.0), "other values stay");
    run_err(&mut s, "image.setTransform", json!({"glyph": "A"}));
    run_err(&mut s, "image.setTransform", json!({"glyph": "A", "width": 0}));
    run_err(&mut s, "image.setTransform", json!({"glyph": "A", "x": 2e6}));
    run_err(&mut s, "image.setTransform", json!({"glyph": "B", "x": 1}));
}

#[test]
fn crops_are_set_reset_and_fitted_to_the_layer_bounds() {
    let dir = folder("crop");
    let file = write(&dir, "A.png", &png(400, 300, None));
    let mut s = Session::with_document();
    run(&mut s, "spacing.setWidth", json!({"glyphs": ["A"], "value": 600}));
    run(&mut s, "glyph.addImage", json!({"files": [file]}));

    let r = run(&mut s, "image.setCrop", json!({"glyph": "A", "rect": [10, 20, 100, 50]}));
    assert_eq!(r["image"]["crop"], json!([10.0, 20.0, 100.0, 50.0]));
    run_err(&mut s, "image.setCrop", json!({"glyph": "A", "rect": [350, 0, 100, 10]}));
    run_err(&mut s, "image.setCrop", json!({"glyph": "A", "rect": [0, 0, 0, 10]}));
    run_err(&mut s, "image.setCrop", json!({"glyph": "A", "rect": [0, 0, 10]}));
    let r = run(&mut s, "image.resetCrop", json!({"glyph": "A"}));
    assert_eq!(r["image"]["crop"], Value::Null);

    // The image covers x −100 to 300 and y −300 to 0; the layer bounds are x 0 to 600 and
    // y −200 (descender) to 800 (ascender): pixels 100 to 400 across, 100 to 300 up.
    run(&mut s, "image.setTransform", json!({"glyph": "A", "x": -100, "y": -300}));
    let r = run(&mut s, "image.setCropToLayerBounds", json!({"glyph": "A"}));
    assert_eq!(r["image"]["crop"], json!([100.0, 100.0, 300.0, 200.0]));
    // Outside the bounds: refused.
    run(&mut s, "image.setTransform", json!({"glyph": "A", "x": 1000}));
    run_err(&mut s, "image.setCropToLayerBounds", json!({"glyph": "A"}));
}

#[test]
fn a_locked_image_refuses_every_edit_until_unlocked() {
    let dir = folder("lock");
    let file = write(&dir, "A.png", &png(400, 300, None));
    let mut s = Session::with_document();
    run(&mut s, "glyph.addImage", json!({"files": [file]}));
    let r = run(&mut s, "image.lock", json!({"glyph": "A"}));
    assert_eq!(r["locked"], json!(true));
    let steps = undo_steps(&s);
    run(&mut s, "image.lock", json!({"glyph": "A"}));
    assert_eq!(undo_steps(&s), steps, "locking a locked image changes nothing");
    assert_eq!(info(&mut s, "A")["locked"], json!(true));
    for (id, params) in [
        ("image.setTransform", json!({"glyph": "A", "x": 5})),
        ("image.setCrop", json!({"glyph": "A", "rect": [0, 0, 10, 10]})),
        ("image.setCropToLayerBounds", json!({"glyph": "A"})),
        ("image.resetCrop", json!({"glyph": "A"})),
        ("glyph.addImage", json!({"files": [file]})),
    ] {
        let e = run_err(&mut s, id, params);
        assert!(e.contains("locked"), "{id}: {e}");
    }
    run(&mut s, "image.unlock", json!({"glyph": "A"}));
    run(&mut s, "image.setTransform", json!({"glyph": "A", "x": 5}));
    assert_eq!(info(&mut s, "A")["x"].as_f64(), Some(5.0));
    // No image: every image command refuses.
    for id in ["image.lock", "image.unlock", "image.resetCrop", "image.setCropToLayerBounds"] {
        run_err(&mut s, id, json!({"glyph": "B"}));
    }
}

#[test]
fn an_unsaved_document_stores_absolute_paths_until_its_first_save() {
    let dir = folder("save");
    let file = write(&dir, "doc/images/A.png", &png(10, 10, None));
    let mut s = Session::with_document();
    run(&mut s, "glyph.addImage", json!({"files": [file]}));
    assert_eq!(info(&mut s, "A")["path"].as_str(), Some(file.as_str()));

    run(&mut s, "file.saveAs", json!({"path": dir.join("doc/Font.fontcraft")}));
    let i = info(&mut s, "A");
    assert_eq!((i["path"].as_str(), i["resolved"].as_bool()), (Some("images/A.png"), Some(true)));
    assert!(!s.doc().unwrap().modified(), "the rewrite is part of the save");

    // Saved into another folder: the path climbs out of it.
    std::fs::create_dir_all(dir.join("other")).unwrap();
    run(&mut s, "file.saveAs", json!({"path": dir.join("other/Font.fontcraft")}));
    let i = info(&mut s, "A");
    assert_eq!((i["path"].as_str(), i["resolved"].as_bool()), (Some("../doc/images/A.png"), Some(true)));

    // A saved document stores new images relative to its folder, also given relative.
    let b = write(&dir, "other/B.png", &png(10, 10, None));
    run(&mut s, "glyph.addImage", json!({"files": [b]}));
    assert_eq!(info(&mut s, "B")["path"].as_str(), Some("B.png"));
    run(&mut s, "glyph.addImage", json!({"files": ["../doc/images/A.png"], "glyphs": ["C"]}));
    assert_eq!(info(&mut s, "C")["resolved"].as_bool(), Some(true));
    // The reopened file keeps the relative paths.
    run(&mut s, "file.save", json!({}));
    let mut t = Session::new();
    run(&mut t, "file.open", json!({"path": dir.join("other/Font.fontcraft")}));
    assert_eq!(info(&mut t, "A")["path"].as_str(), Some("../doc/images/A.png"));
    assert_eq!(info(&mut t, "A")["resolved"].as_bool(), Some(true));
}
