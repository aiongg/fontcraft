//! Autotrace (M13.30): `glyph.autotraceImage` traces a placed image into the layer's paths. The
//! images are rendered in code (`fontcraft_render`) and written as PNG files.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;

use fontcraft_engine::Session;
use fontcraft_geom::{Affine, BezPath, Point, Rect, Shape as _};
use fontcraft_model::Shape;
use fontcraft_pathops::coverage_difference;
use fontcraft_render::{Style, render_path};
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

fn folder(tag: &str) -> PathBuf {
    let dir = temp_dir("autotrace").join(tag);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `path` (y up) rendered black on white into a `width × height` PNG file in `dir`.
fn png_of(dir: &std::path::Path, name: &str, path: &BezPath, width: u32, height: u32) -> String {
    let flip = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, f64::from(height)]);
    let image = render_path(path, flip, width, height, &Style::default()).unwrap();
    let file = dir.join(name);
    std::fs::write(&file, image.png().unwrap()).unwrap();
    file.to_string_lossy().into_owned()
}

/// A PNG header claiming `width × height` pixels and no pixel data: placing reads only this.
fn png_header(width: u32, height: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    b.extend((ihdr.len() as u32).to_be_bytes());
    b.extend(b"IHDR");
    b.extend(&ihdr);
    b.extend([0; 4]);
    b.extend([0, 0, 0, 0]);
    b.extend(b"IEND");
    b.extend([0; 4]);
    b
}

/// A circle of four cubic arcs, counter-clockwise unless `clockwise`.
fn circle(path: &mut BezPath, c: Point, r: f64, clockwise: bool) {
    let k = 0.552_284_75 * r;
    let s = if clockwise { -1.0 } else { 1.0 };
    let p = |x: f64, y: f64| Point::new(c.x + x, c.y + s * y);
    path.move_to(p(r, 0.0));
    path.curve_to(p(r, k), p(k, r), p(0.0, r));
    path.curve_to(p(-k, r), p(-r, k), p(-r, 0.0));
    path.curve_to(p(-r, -k), p(-k, -r), p(0.0, -r));
    path.curve_to(p(k, -r), p(r, -k), p(r, 0.0));
    path.close_path();
}

/// The outlines of the paths of `glyph`'s first-master layer, from shape `from` on.
fn paths_of(s: &Session, glyph: &str, from: usize) -> Vec<BezPath> {
    let font = s.font().unwrap();
    let layer = font.glyph(glyph).unwrap().layers.iter().find(|l| l.id == "m01").unwrap();
    layer
        .shapes
        .iter()
        .skip(from)
        .filter_map(|sh| match sh {
            Shape::Path(p) => Some(p.to_bez().unwrap()),
            _ => None,
        })
        .collect()
}

fn shape_count(s: &Session, glyph: &str) -> usize {
    s.font().unwrap().glyph(glyph).unwrap().layers.iter().find(|l| l.id == "m01").map_or(0, |l| l.shapes.len())
}

#[test]
fn autotrace_image_traces_a_rendered_o_back_within_one_percent_of_its_area() {
    let dir = folder("o");
    let mut o = BezPath::new();
    circle(&mut o, Point::new(400.0, 400.0), 350.0, false);
    circle(&mut o, Point::new(400.0, 400.0), 180.0, true);
    let file = png_of(&dir, "O.png", &o, 800, 800);
    let mut s = Session::with_document();
    let placed = run(&mut s, "glyph.addImage", json!({"files": [file], "glyphs": ["A"]}));
    assert_eq!(placed["placed"][0]["scale"], json!([1.0, 1.0]), "one pixel per unit");
    let before = shape_count(&s, "A");
    let steps = undo_steps(&s);

    let r = run(&mut s, "glyph.autotraceImage", json!({"glyphs": ["A"], "removeImage": true}));
    assert_eq!(r["traced"], json!([{"glyph": "A", "layer": "m01", "contours": 2, "imageRemoved": true}]));
    assert_eq!(r["skipped"], json!([]));
    assert_eq!(undo_steps(&s), steps + 1, "one undo step");
    assert_eq!(run(&mut s, "image.getInfo", json!({"glyph": "A"}))["image"], Value::Null, "the image is gone");

    let traced = paths_of(&s, "A", before);
    assert_eq!(traced.len(), 2);
    let diff = coverage_difference(&[o], &traced).unwrap();
    assert!(diff.total <= 0.01 * diff.ink, "{} of {} square units differ", diff.total, diff.ink);
    let nodes: usize = traced.iter().map(|p| p.elements().len()).sum();
    assert!(nodes < 100, "a few curves, not pixel steps: {nodes} elements");
}

#[test]
fn autotrace_image_places_the_outline_where_the_image_is_drawn() {
    let dir = folder("placed");
    let rect = Rect::new(10.0, 5.0, 30.0, 15.0).to_path(0.1);
    let file = png_of(&dir, "bar.png", &rect, 40, 20);
    let mut s = Session::with_document();
    run(&mut s, "glyph.addImage", json!({"files": [file], "glyphs": ["B"]}));
    // Twice the size, its corner at (100, 50).
    run(&mut s, "image.setTransform", json!({"glyph": "B", "x": 100, "y": 50, "width": 80, "height": 40}));
    let before = shape_count(&s, "B");

    // No `glyphs`: every glyph with an image on the layer, here B alone.
    let r = run(&mut s, "glyph.autotraceImage", json!({}));
    assert_eq!(r["traced"], json!([{"glyph": "B", "layer": "m01", "contours": 1, "imageRemoved": false}]));
    let traced = paths_of(&s, "B", before);
    assert_eq!(traced.len(), 1);
    let bounds = traced[0].bounding_box();
    assert_eq!((bounds.x0, bounds.y0, bounds.x1, bounds.y1), (120.0, 60.0, 160.0, 80.0));
    assert!(traced[0].area() > 0.0, "an outer contour runs counter-clockwise");
    assert!(run(&mut s, "image.getInfo", json!({"glyph": "B"}))["image"].is_object(), "the image stays");

    // A glyph without an image is listed beside the one traced.
    let r = run(&mut s, "glyph.autotraceImage", json!({"glyphs": ["B", "C"]}));
    assert_eq!(r["traced"][0]["glyph"], "B");
    assert_eq!(r["skipped"], json!([{"glyph": "C", "layer": "m01", "reason": "the layer has no image"}]));
}

#[test]
fn autotrace_image_refuses_and_changes_nothing_when_nothing_can_be_traced() {
    let dir = folder("refused");
    let mut s = Session::with_document();
    let e = run_err(&mut s, "glyph.autotraceImage", json!({}));
    assert!(e.contains("no glyph has an image"), "{e}");
    let e = run_err(&mut s, "glyph.autotraceImage", json!({"glyphs": ["A"]}));
    assert!(e.contains("no image"), "{e}");

    // A blank image has no ink.
    let blank = png_of(&dir, "blank.png", &BezPath::new(), 30, 30);
    run(&mut s, "glyph.addImage", json!({"files": [blank], "glyphs": ["A"]}));
    let e = run_err(&mut s, "glyph.autotraceImage", json!({"glyphs": ["A"]}));
    assert!(e.contains("darker than the threshold"), "{e}");

    // Over 64 megapixels: refused from the header, before anything is decoded.
    let huge = dir.join("huge.png");
    std::fs::write(&huge, png_header(9_000, 8_000)).unwrap();
    run(&mut s, "glyph.addImage", json!({"files": [huge.to_string_lossy()], "glyphs": ["B"]}));
    let e = run_err(&mut s, "glyph.autotraceImage", json!({"glyphs": ["B"]}));
    assert!(e.contains("pixels"), "{e}");

    for bad in [json!({"threshold": 0}), json!({"threshold": 1.5}), json!({"smoothness": -1}), json!({"glyphs": []}), json!({"removeImage": "yes"})] {
        run_err(&mut s, "glyph.autotraceImage", bad);
    }
}
