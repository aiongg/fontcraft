//! PDF outlines and images in the engine: `edit.paste` of the PDF clipboard flavour,
//! `file.importOutlines` of a PDF file, and `glyph.autotraceImage` and `bitmap.setStrikes` of a
//! placed PDF. The PDFs are written in code.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use base64::Engine as _;
use fontcraft_engine::Session;
use fontcraft_engine::model::{Dimension, Path, Shape};
use fontcraft_testkit::{snapshot, temp_dir};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn run_err(s: &mut Session, id: &str, p: Value) -> String {
    let before = snapshot(s);
    let e = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} succeeded")).to_string();
    assert_eq!(snapshot(s), before, "{id} failed but changed the session");
    e
}

/// A PDF of `objects` (object 1 the catalog) with a classic cross-reference table.
fn raw_pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.5\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

fn stream(dict: &str, content: &str) -> Vec<u8> {
    format!("<< {dict} /Length {} >>\nstream\n{content}\nendstream", content.len()).into_bytes()
}

/// One page with `media_box` drawing `content`.
fn one_page(media_box: &str, content: &str) -> Vec<u8> {
    raw_pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [{media_box}] >>").into_bytes(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        stream("", content),
    ])
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn paths(s: &Session, glyph: &str) -> Vec<Path> {
    let layer = s.font().unwrap().glyph(glyph).unwrap().layer("m01").unwrap().shapes.clone();
    layer
        .into_iter()
        .filter_map(|sh| match sh {
            Shape::Path(p) => Some(p),
            _ => None,
        })
        .collect()
}

fn points(p: &Path) -> Vec<(f64, f64)> {
    p.nodes.iter().map(|n| (n.x, n.y)).collect()
}

#[test]
fn paste_pdf_flavour_adds_the_pages_outlines() {
    // A filled square and a stroked line 6 points wide, on a page whose box starts at (50, 20).
    let pdf = one_page("50 20 250 220", "60 30 40 40 re f\n6 w 100 100 m 150 100 l S");
    let mut s = Session::with_document();
    let v = run(&mut s, "edit.paste", json!({"glyph": "A", "pdf": b64(&pdf)}));
    assert_eq!(v["flavour"], "pdf");
    assert_eq!(v["pasted"], 2);
    assert_eq!(v["selection"]["shapes"], json!([0, 1]));

    // One unit per point, the page's bottom-left corner on the origin.
    let pasted = paths(&s, "A");
    assert_eq!(pasted.len(), 2);
    assert!(pasted[0].closed);
    assert_eq!(points(&pasted[0]), vec![(10.0, 10.0), (50.0, 10.0), (50.0, 50.0), (10.0, 50.0)]);
    assert!(!pasted[1].closed);
    assert_eq!(points(&pasted[1]), vec![(50.0, 80.0), (100.0, 80.0)]);
    assert_eq!(pasted[1].info.attrs.stroke_width, Some(Dimension::Number(6.0)));
    run(&mut s, "edit.undo", json!({}));
    assert!(paths(&s, "A").is_empty(), "the paste is one undo step");

    // `at` puts the bottom left of the pasted box there; JSON that cannot be read gives way.
    let v = run(&mut s, "edit.paste", json!({"glyph": "A", "json": "not a fragment", "pdf": b64(&pdf), "at": [100, 0]}));
    assert_eq!(v["flavour"], "pdf");
    assert_eq!(v["bounds"], json!([100.0, 0.0, 190.0, 70.0]));
}

#[test]
fn paste_pdf_refuses_hostile_or_oversized_data_and_changes_nothing() {
    let mut s = Session::with_document();
    let e = run_err(&mut s, "edit.paste", json!({"glyph": "A", "pdf": "%%% not base64"}));
    assert!(e.contains("base64"), "{e}");
    let e = run_err(&mut s, "edit.paste", json!({"glyph": "A", "pdf": b64(b"GIF89a")}));
    assert!(e.contains("not a PDF"), "{e}");
    let e = run_err(&mut s, "edit.paste", json!({"glyph": "A", "pdf": b64(b"%PDF-1.5\n1 0 obj\n<< /Type")}));
    assert!(e.contains("PDF cannot be pasted"), "{e}");

    // A form that draws itself.
    let recursive = raw_pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /XObject << /F 5 0 R >> >> >>".to_vec(),
        stream("", "/F Do"),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /F 5 0 R >> >>", "0 0 1 1 re f /F Do"),
    ]);
    let e = run_err(&mut s, "edit.paste", json!({"glyph": "A", "pdf": b64(&recursive)}));
    assert!(e.contains("draws itself"), "{e}");

    // Refused from its length, before it is decoded.
    let huge = "A".repeat((64 << 20) / 3 * 4 + 8);
    let e = run_err(&mut s, "edit.paste", json!({"glyph": "A", "pdf": huge}));
    assert!(e.contains("64 MiB"), "{e}");

    // A page with nothing painted has nothing to paste.
    let e = run_err(&mut s, "edit.paste", json!({"glyph": "A", "pdf": b64(&one_page("0 0 10 10", "BT ET"))}));
    assert!(e.contains("nothing to paste"), "{e}");
}

#[test]
fn import_outlines_reads_a_pdf_file_with_its_page_top_at_the_ascender() {
    let dir = temp_dir("clipboard_pdf").join("import");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("A.pdf");
    std::fs::write(&file, one_page("0 0 200 200", "10 20 30 40 re f")).unwrap();
    let file = file.to_string_lossy().into_owned();
    let mut s = Session::with_document();

    let v = run(&mut s, "file.importOutlines", json!({"files": [file.clone()]}));
    assert_eq!(v["imported"][0]["glyph"], "A", "{v}");
    assert_eq!(v["imported"][0]["paths"], 1);
    // The page top (200) at the ascender (800), one unit per point.
    assert_eq!(points(&paths(&s, "A")[0]), vec![(10.0, 620.0), (40.0, 620.0), (40.0, 660.0), (10.0, 660.0)]);

    // `em` fits the page's height (200) to the em (1000), its top at the ascender.
    run(&mut s, "file.importOutlines", json!({"files": [file], "glyphs": ["B"], "scale": "em"}));
    assert_eq!(points(&paths(&s, "B")[0]), vec![(50.0, -100.0), (200.0, -100.0), (200.0, 100.0), (50.0, 100.0)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn autotrace_traces_a_placed_pdf_from_its_drawn_page() {
    let dir = temp_dir("clipboard_pdf").join("trace");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("bar.pdf");
    std::fs::write(&file, one_page("0 0 200 100", "50 20 100 40 re f")).unwrap();
    let mut s = Session::with_document();
    run(&mut s, "glyph.addImage", json!({"files": [file.to_string_lossy()], "glyphs": ["A"]}));
    run(&mut s, "image.setTransform", json!({"glyph": "A", "x": 0, "y": 0, "width": 200, "height": 100}));

    let r = run(&mut s, "glyph.autotraceImage", json!({"glyphs": ["A"]}));
    assert_eq!(r["traced"], json!([{"glyph": "A", "layer": "m01", "contours": 1, "imageRemoved": false}]), "{r}");
    let traced = &paths(&s, "A")[0];
    let (xs, ys): (Vec<f64>, Vec<f64>) = traced.nodes.iter().map(|n| (n.x, n.y)).unzip();
    let min = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
    let max = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    // The page is drawn at several pixels per point, and the outline lands in points.
    let bounds = [min(&xs), min(&ys), max(&xs), max(&ys)];
    for (got, want) in bounds.iter().zip([50.0, 20.0, 150.0, 60.0]) {
        assert!((got - want).abs() <= 1.0, "{bounds:?}");
    }

    // A crop, in points, limits the trace to its part of the page.
    run(&mut s, "image.setCrop", json!({"glyph": "A", "rect": [0, 0, 100, 100]}));
    let before = paths(&s, "A").len();
    run(&mut s, "glyph.autotraceImage", json!({"glyphs": ["A"]}));
    let cropped = &paths(&s, "A")[before];
    let right = cropped.nodes.iter().map(|n| n.x).fold(f64::NEG_INFINITY, f64::max);
    assert!((right - 100.0).abs() <= 1.0, "{right}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_new_strike_draws_the_placed_pdf_of_a_glyph_without_outlines() {
    let dir = temp_dir("clipboard_pdf").join("strike");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("corner.pdf");
    // A 100-point page, black in its bottom-left quarter.
    std::fs::write(&file, one_page("0 0 100 100", "0 0 0 rg 0 0 50 50 re f")).unwrap();
    let mut s = Session::with_document();
    run(&mut s, "glyph.addImage", json!({"files": [file.to_string_lossy()], "glyphs": ["A"]}));

    // One point per unit at 100 pixels per em of 1000 units: the page is 10 pixels square.
    let r = run(&mut s, "bitmap.setStrikes", json!({"add": [{"ppem": 100}]}));
    assert!(r.get("warnings").is_none(), "{r}");
    let font = s.font().unwrap();
    let a = font.bitmap_strikes[0].glyph("A").unwrap();
    assert_eq!((a.width, a.height, a.left, a.bottom), (10, 10, 0, 0));
    // Rows top to bottom, two bytes each: the bottom five rows have their left five pixels on.
    let rows: Vec<&[u8]> = a.data.chunks(2).collect();
    assert_eq!(rows.len(), 10);
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(*row, if i < 5 { &[0u8, 0][..] } else { &[0xF8, 0][..] }, "row {i}");
    }
    // A glyph with an outline keeps the outline's bitmap, and glyphs without either stay empty.
    let notdef = font.bitmap_strikes[0].glyph(".notdef").unwrap();
    assert_eq!((notdef.width, notdef.left), (40, 5));
    assert!(font.bitmap_strikes[0].glyph("B").unwrap().data.is_empty());

    // A placed PDF too large for a bitmap is left out with a warning; the strike is still added.
    run(&mut s, "image.setTransform", json!({"glyph": "A", "x": 0, "y": 0, "width": 20000, "height": 20000}));
    let r = run(&mut s, "bitmap.setStrikes", json!({"add": [{"ppem": 101}]}));
    let warning = r["warnings"][0].as_str().unwrap_or_default().to_string();
    assert!(warning.contains("glyph `A`") && warning.contains("at most 1024"), "{r}");
    assert!(s.font().unwrap().bitmap_strikes.iter().find(|st| st.ppem == 101).unwrap().glyph("A").unwrap().data.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
