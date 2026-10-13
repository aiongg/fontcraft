//! Exported TrueType point numbers (M10.19): `hint.tt.getPointIndexes` names the points the
//! exported `glyf` outline has, through overlap removal.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_engine::model::{Path, Shape};
use serde_json::json;
use skrifa::raw::TableProvider;
use skrifa::raw::tables::glyf::Glyph;
use skrifa::{FontRef, GlyphId};

#[test]
fn point_indexes_match_the_exported_outline() {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let mut g = font.glyph_mut("I")?;
        let layer = g.layer_mut("m01").unwrap();
        // Two overlapping squares: the corner of each inside the other is removed at export.
        layer.shapes = vec![Shape::Path(Path::rect(0.0, 0.0, 200.0, 200.0)), Shape::Path(Path::rect(100.0, 100.0, 300.0, 300.0))];
        Ok(())
    })
    .unwrap();
    let out = s.execute("hint.tt.getPointIndexes", &json!({"glyph": "I"})).unwrap();
    assert_eq!(out["composite"], json!(false));
    let count = usize::try_from(out["count"].as_u64().unwrap()).unwrap();
    assert_eq!(count, 8, "the union of the squares has eight corners: {out}");

    let font = s.font().unwrap().clone();
    let compiled = fontcraft_compile::compile_ttf(&font, &fontcraft_compile::Options::default()).unwrap();
    let file = FontRef::new(&compiled.bytes).unwrap();
    let gid = GlyphId::new(u32::try_from(compiled.glyph_order.iter().position(|n| n == "I").unwrap()).unwrap());
    let Some(Glyph::Simple(glyph)) = file.loca(None).unwrap().get_glyf(gid, &file.glyf().unwrap()).unwrap() else { panic!("I is a simple glyph") };
    let points: Vec<_> = glyph.points().collect();
    assert_eq!(points.len(), count);
    let layer = font.glyph("I").unwrap().layer("m01").unwrap().clone();
    let paths: Vec<&Path> = layer.shapes.iter().filter_map(|s| if let Shape::Path(p) = s { Some(p) } else { None }).collect();
    for (pi, path) in paths.iter().enumerate() {
        for (ni, n) in path.nodes.iter().enumerate() {
            let index = usize::try_from(out["paths"][pi][ni].as_u64().unwrap_or_else(|| panic!("node ({pi}, {ni}) has a point"))).unwrap();
            let p = points[index];
            assert!(p.on_curve);
            let within = |lo: f64, hi: f64| n.x > lo && n.x < hi && n.y > lo && n.y < hi;
            let inside = within(0.0, 200.0) || within(100.0, 300.0);
            if inside {
                assert!((p.x, p.y) == (200, 100) || (p.x, p.y) == (100, 200), "a removed node takes the nearest crossing: {p:?}");
            } else {
                assert_eq!((f64::from(p.x), f64::from(p.y)), (n.x, n.y), "node ({pi}, {ni})");
            }
        }
    }
    let refused = s.execute("hint.tt.getPointIndexes", &json!({"glyph": "nope"})).expect_err("unknown glyph");
    assert!(refused.to_string().contains("nope"));
}
