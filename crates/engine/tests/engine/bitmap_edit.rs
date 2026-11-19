//! Bitmap glyph editing (`bitmap.setPixels`, `bitmap.transform`, `bitmap.regenerateGlyphs`,
//! `bitmap.removeGlyphs`, `bitmap.setStrikeProperties`): pixels, rows and offsets set with undo,
//! each transform's pixels and box, references drawn in on export with loops refused, regenerated
//! bitmaps replacing hand edits, and the advance editable only in a font without outlines.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{BitmapStrike, StrikeGlyph};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs a command that must fail, checks that the font is the same object as before, and
/// returns the message.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: a refused command changed the font");
    e
}

fn strikes(s: &Session) -> Vec<BitmapStrike> {
    s.font().unwrap().bitmap_strikes.to_vec()
}

fn bitmap(s: &Session, strike: usize, glyph: &str) -> StrikeGlyph {
    strikes(s)[strike].glyph(glyph).unwrap_or_else(|| panic!("no bitmap for {glyph}")).clone()
}

/// A new document (whose glyphs are 600 units wide, so 10 pixels at 16 ppem) with `A` and `B`
/// drawn as 500-unit squares, and empty strikes of the given sizes and depths.
fn outlined(strikes: &[(u16, u8)]) -> Session {
    let mut s = Session::with_document();
    for glyph in ["A", "B"] {
        run(&mut s, "path.addPath", json!({"glyph": glyph, "nodes": fontcraft_testkit::rect(0.0, 0.0, 500.0, 500.0), "closed": true}));
    }
    let add: Vec<Value> = strikes.iter().map(|(ppem, depth)| json!({"ppem": ppem, "depth": depth})).collect();
    run(&mut s, "bitmap.setStrikes", json!({"add": add, "rasterize": false}));
    s
}

fn rows(s: &mut Session, glyph: &str, ppem: u16, rows: &[&str], origin: [i32; 2]) {
    run(s, "bitmap.setPixels", json!({"glyph": glyph, "strike": {"ppem": ppem}, "rows": rows, "origin": origin}));
}

#[test]
fn pixels_rows_and_origin_are_set_with_undo_and_the_bitmap_grows_to_a_set_pixel() {
    let mut s = outlined(&[(16, 1)]);
    let out = run(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 16}, "rows": ["#.", ".#"], "origin": [1, 2]}));
    assert_eq!(out["created"], json!(true));
    let a = bitmap(&s, 0, "A");
    assert_eq!((a.width, a.height, a.left, a.bottom), (2, 2, 1, 2));
    assert_eq!(a.data, vec![0x80, 0x40]);
    assert_eq!(a.advance, 10, "the advance follows the outline width, 600 units at 16 ppem");

    // A pixel right of the box widens it; the old pixels keep their place.
    run(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 16}, "pixels": [{"x": 5, "y": 2, "value": 1}]}));
    let a = bitmap(&s, 0, "A");
    assert_eq!((a.width, a.height, a.left, a.bottom), (5, 2, 1, 2));
    assert_eq!(a.data, vec![0x80, 0x48]);

    // Clearing keeps the box.
    run(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 16}, "pixels": [{"x": 1, "y": 3, "value": 0}]}));
    let a = bitmap(&s, 0, "A");
    assert_eq!((a.width, a.data.clone()), (5, vec![0x00, 0x48]));

    let e = refused(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 16}, "pixels": [{"x": 1100, "y": 0, "value": 1}]}));
    assert!(e.contains("at most 1024 on a side"), "{e}");
    let e = refused(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 16}, "pixels": [{"x": 0, "y": 0, "value": 2}]}));
    assert!(e.contains("`value` from 0 to 1"), "{e}");
    let e = refused(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 16}, "rows": ["#.", "#"]}));
    assert!(e.contains("row 1 is 1 pixels long"), "{e}");
    let e = refused(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 16}, "advance": 3}));
    assert!(e.contains("follows its glyph's width"), "{e}");
    refused(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 12}, "rows": ["#"]}));
    refused(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 16}}));

    for _ in 0..3 {
        run(&mut s, "edit.undo", json!({}));
    }
    assert!(strikes(&s)[0].glyph("A").is_none(), "undo removes the bitmap the first call made");
}

#[test]
fn grey_rows_take_hex_levels() {
    let mut s = outlined(&[(16, 4), (16, 8)]);
    let e = refused(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 16}, "rows": ["#"]}));
    assert!(e.contains("give `depth`"), "{e}");
    run(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 16, "depth": 4}, "rows": ["0F", "8."]}));
    assert_eq!(bitmap(&s, 0, "A").data, vec![0x0F, 0x80]);
    run(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 16, "depth": 8}, "rows": ["ff..", "##7f"]}));
    assert_eq!(bitmap(&s, 1, "A").data, vec![0xFF, 0x00, 0xFF, 0x7F]);
    let e = refused(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 16, "depth": 8}, "rows": ["f"]}));
    assert!(e.contains("2 character(s)"), "{e}");
    run(&mut s, "bitmap.setPixels", json!({"glyph": "B", "strike": {"ppem": 16, "depth": 4}, "pixels": [{"x": 0, "y": 0, "value": 9}]}));
    let b = bitmap(&s, 0, "B");
    assert_eq!((b.width, b.height, b.data.clone()), (1, 1, vec![0x90]));
}

/// The L-shaped bitmap `#..` over `###`, 3 × 2 pixels, transformed by `op` from a fresh start.
fn l_shape(op: Value, origin: [i32; 2]) -> StrikeGlyph {
    let mut s = outlined(&[(8, 1)]);
    rows(&mut s, "A", 8, &["#..", "###"], origin);
    let mut params = json!({"strike": {"ppem": 8}, "glyphs": ["A"]});
    for (k, v) in op.as_object().unwrap() {
        params[k] = v.clone();
    }
    let out = run(&mut s, "bitmap.transform", params);
    assert_eq!(out["glyphs"], json!(1));
    bitmap(&s, 0, "A")
}

#[test]
fn each_transform_moves_the_pixels_and_places_the_box() {
    let flip_h = l_shape(json!({"op": "flipH"}), [0, 0]);
    assert_eq!((flip_h.width, flip_h.height, flip_h.left, flip_h.bottom, flip_h.data), (3, 2, 0, 0, vec![0x20, 0xE0]));
    let flip_v = l_shape(json!({"op": "flipV"}), [0, 0]);
    assert_eq!(flip_v.data, vec![0xE0, 0x80]);
    let half = l_shape(json!({"op": "rotate180"}), [0, 0]);
    assert_eq!(half.data, vec![0xE0, 0x20]);

    // A quarter turn swaps the sides and keeps the box's centre, rounded down.
    let cw = l_shape(json!({"op": "rotate90cw"}), [0, 0]);
    assert_eq!((cw.width, cw.height, cw.left, cw.bottom, cw.data), (2, 3, 0, -1, vec![0xC0, 0x80, 0x80]));
    let ccw = l_shape(json!({"op": "rotate90ccw"}), [0, 0]);
    assert_eq!((ccw.width, ccw.height, ccw.data), (2, 3, vec![0x40, 0x40, 0xC0]));

    // One pixel right per row above the baseline: the baseline row stays.
    let skew = l_shape(json!({"op": "skew", "skew": [1, 1]}), [0, 0]);
    assert_eq!((skew.width, skew.left, skew.data), (4, 0, vec![0x40, 0xE0]));
    // One pixel per two rows: the row below the baseline moves left.
    let below = l_shape(json!({"op": "skew", "skew": [1, 2]}), [0, -1]);
    assert_eq!((below.width, below.left, below.bottom, below.data), (4, -1, -1, vec![0x40, 0xE0]));

    let mut s = outlined(&[(8, 1)]);
    rows(&mut s, "A", 8, &["#"], [0, 0]);
    refused(&mut s, "bitmap.transform", json!({"strike": {"ppem": 8}, "op": "skew"}));
    refused(&mut s, "bitmap.transform", json!({"strike": {"ppem": 8}, "op": "skew", "skew": [1, 0]}));
    refused(&mut s, "bitmap.transform", json!({"strike": {"ppem": 8}, "op": "flipH", "skew": [1, 1]}));
    refused(&mut s, "bitmap.transform", json!({"strike": {"ppem": 8}, "op": "rotate45"}));
    let e = refused(&mut s, "bitmap.transform", json!({"strike": {"ppem": 8}, "op": "flipH", "glyphs": ["B"]}));
    assert!(e.contains("none of the glyphs has a bitmap"), "{e}");
    let out = run(&mut s, "bitmap.transform", json!({"strike": {"ppem": 8}, "op": "flipH", "glyphs": ["A", "B"]}));
    assert_eq!(out["skipped"], json!(["B"]));
}

#[test]
fn references_draw_in_on_export_and_a_reference_loop_is_refused() {
    let mut s = outlined(&[(8, 1)]);
    rows(&mut s, "B", 8, &["#"], [0, 0]);
    run(
        &mut s,
        "bitmap.setPixels",
        json!({"glyph": "A", "strike": {"ppem": 8}, "rows": ["#"], "origin": [2, 0], "references": [{"glyph": "B", "y": 3}]}),
    );
    assert_eq!(bitmap(&s, 0, "A").references.len(), 1);

    // B → A would lead back to B.
    let e = refused(&mut s, "bitmap.setPixels", json!({"glyph": "B", "strike": {"ppem": 8}, "references": [{"glyph": "A"}]}));
    assert!(e.contains("lead back"), "{e}");
    refused(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 8}, "references": [{"glyph": "A"}]}));
    refused(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 8}, "references": [{"glyph": "C"}]}));

    // A's own pixel at (2, 0) and B's drawn at (0, 3).
    let composed = strikes(&s)[0].composed("A").unwrap().unwrap();
    assert_eq!((composed.width, composed.height, composed.left, composed.bottom), (3, 4, 0, 0));
    assert_eq!(composed.data, vec![0x80, 0x00, 0x00, 0x20]);
    assert!(composed.references.is_empty());

    let dir = std::env::temp_dir().join(format!("fontcraft-bitmap-edit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("refs.bdf");
    run(&mut s, "bitmap.exportStrikes", json!({"format": "bdf", "path": path.to_string_lossy()}));
    let text = std::fs::read_to_string(&path).unwrap();
    let a = text.split("STARTCHAR A\n").nth(1).unwrap();
    assert!(a.contains("BBX 3 4 0 0\nBITMAP\n80\n00\n00\n20\n"), "{a}");
    std::fs::remove_dir_all(&dir).unwrap();

    // Renaming B carries the reference along; removing B's bitmap removes it.
    run(&mut s, "glyph.rename", json!({"glyph": "B", "newName": "B.base"}));
    assert_eq!(bitmap(&s, 0, "A").references[0].glyph, "B.base");
    let out = run(&mut s, "bitmap.removeGlyphs", json!({"glyphs": ["B.base"]}));
    assert_eq!((out["removed"].clone(), out["unlinked"].clone()), (json!(1), json!(["A"])));
    assert!(bitmap(&s, 0, "A").references.is_empty());
    let e = refused(&mut s, "bitmap.removeGlyphs", json!({"glyphs": ["B.base"]}));
    assert!(e.contains("none of the glyphs has a bitmap"), "{e}");
    refused(&mut s, "bitmap.removeGlyphs", json!({}));
}

#[test]
fn regenerating_replaces_hand_edits_of_the_chosen_glyphs_only() {
    let mut s = outlined(&[(16, 1)]);
    rows(&mut s, "A", 16, &["#"], [3, 3]);
    rows(&mut s, "B", 16, &["#"], [3, 3]);
    run(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 16}, "references": [{"glyph": "B"}]}));
    let out = run(&mut s, "bitmap.regenerateGlyphs", json!({"glyphs": ["A"]}));
    assert_eq!((out["strikes"].clone(), out["glyphs"].clone()), (json!(["16"]), json!(1)));
    let a = bitmap(&s, 0, "A");
    assert_eq!((a.width, a.height, a.left, a.bottom, a.advance), (8, 8, 0, 0, 10), "500 units at 16 ppem");
    assert_eq!(a.data, vec![0xFF; 8]);
    assert!(a.references.is_empty());
    let b = bitmap(&s, 0, "B");
    assert_eq!((b.width, b.left), (1, 3), "B keeps its hand edit");
    refused(&mut s, "bitmap.regenerateGlyphs", json!({"strikes": [{"ppem": 12}]}));
}

#[test]
fn the_advance_is_editable_only_in_a_font_without_outlines() {
    let mut s = Session::with_document();
    // The new document's `.notdef` is its only outline.
    run(&mut s, "glyph.removeGlyph", json!({"glyphs": [".notdef"]}));
    run(&mut s, "bitmap.setStrikes", json!({"add": [{"ppem": 10}], "rasterize": false}));
    let out = run(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 10}, "rows": ["#"]}));
    assert_eq!(out["advance"], json!(6), "a new bitmap starts from the width, 600 units at 10 ppem");
    run(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 10}, "advance": 7}));
    assert_eq!(bitmap(&s, 0, "A").advance, 7);
    run(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 10}, "pixels": [{"x": 0, "y": 1, "value": 1}]}));
    assert_eq!(bitmap(&s, 0, "A").advance, 7, "a pixel edit keeps a hand-set advance");
}

#[test]
fn strike_properties_are_set_as_one_step() {
    let mut s = outlined(&[(12, 1)]);
    let out = run(
        &mut s,
        "bitmap.setStrikeProperties",
        json!({"strike": {"ppem": 12}, "name": "-Test-Pixel-Medium-R-Normal--12-120-75-75-C-60-ISO10646-1", "pointSize": 12, "resolution": [75, 75], "ascent": 10, "descent": 2, "properties": [{"name": "WEIGHT_NAME", "value": "Medium"}, {"name": "PIXEL_SIZE", "value": 12}]}),
    );
    assert_eq!(out["properties"], json!(2));
    let strike = &strikes(&s)[0];
    assert_eq!((strike.point_size, strike.resolution, strike.ascent, strike.descent), (12.0, [75, 75], 10, 2));
    assert_eq!(strike.properties[1].name, "PIXEL_SIZE");
    let e = refused(
        &mut s,
        "bitmap.setStrikeProperties",
        json!({"strike": {"ppem": 12}, "properties": [{"name": "A", "value": 1}, {"name": "A", "value": 2}]}),
    );
    assert!(e.contains("given twice"), "{e}");
    refused(&mut s, "bitmap.setStrikeProperties", json!({"strike": {"ppem": 12}, "properties": [{"name": "A", "value": 1.5}]}));
    refused(&mut s, "bitmap.setStrikeProperties", json!({"strike": {"ppem": 12}, "pointSize": -1}));
    refused(&mut s, "bitmap.setStrikeProperties", json!({"strike": {"ppem": 12}}));
    run(&mut s, "edit.undo", json!({}));
    assert!(strikes(&s)[0].properties.is_empty());
}

#[test]
fn references_nest_at_most_eight_deep() {
    let mut s = outlined(&[(8, 1)]);
    rows(&mut s, "J", 8, &["#"], [0, 0]);
    // I refers to J, H to I, and so on: B's chain is 8 deep.
    let chain = ["J", "I", "H", "G", "F", "E", "D", "C", "B"];
    for pair in chain.windows(2) {
        run(&mut s, "bitmap.setPixels", json!({"glyph": pair[1], "strike": {"ppem": 8}, "references": [{"glyph": pair[0], "x": 1}]}));
    }
    assert_eq!(strikes(&s)[0].composed("B").unwrap().unwrap().left, 8);
    let e = refused(&mut s, "bitmap.setPixels", json!({"glyph": "A", "strike": {"ppem": 8}, "references": [{"glyph": "B"}]}));
    assert!(e.contains("more than 8 levels deep"), "{e}");
}
